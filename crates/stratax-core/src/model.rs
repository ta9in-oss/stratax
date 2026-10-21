//! Shared data model. Everything the UIs and the AI advisor see is defined here.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// How risky it is to reclaim a finding (labels borrowed from SpaceGuard).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    /// Pure cache; nothing is lost.
    Safe,
    /// Comes back after a rebuild / reinstall / re-download.
    Rebuild,
    /// May hold state the user cares about. Never preselected.
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    AgentSession,
    AgentData,
    Worktree,
    MutationTest,
    BuildArtifact,
    PackageCache,
    Toolchain,
    BrowserData,
    AppCache,
    Container,
    Models,
    Trash,
    System,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::AgentSession => "Agent sessions",
            Category::AgentData => "Agent data",
            Category::Worktree => "Worktrees",
            Category::MutationTest => "Mutation tests",
            Category::BuildArtifact => "Build artifacts",
            Category::PackageCache => "Package caches",
            Category::Toolchain => "Toolchains",
            Category::BrowserData => "Browsers",
            Category::AppCache => "App caches",
            Category::Container => "Containers",
            Category::Models => "AI models",
            Category::Trash => "Trash",
            Category::System => "System",
        }
    }
}

/// What reclaiming a finding means.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Remove the path (or only listed sub-paths).
    Delete { paths: Vec<PathBuf> },
    /// Hand off to the owning tool (docker, snap, cargo...).
    RunTool { program: String, args: Vec<String> },
    /// `git worktree remove` from the main repository.
    WorktreeRemove { repo: PathBuf },
    /// Move to a larger / slower disk and leave a symlink behind.
    Relocate { suggested_root: PathBuf },
    /// Nothing automatic; the user decides.
    ReviewOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Stable id (hash of rule + path) so the advisor can refer to it.
    pub id: String,
    pub rule: String,
    pub title: String,
    pub path: PathBuf,
    pub category: Category,
    /// Which agent / tool produced it ("claude-code", "cargo-mutants", ...).
    pub owner: Option<String>,
    pub safety: Safety,
    pub bytes: u64,
    pub files: u64,
    /// Newest mtime inside, unix seconds.
    pub last_activity: Option<i64>,
    pub reason: String,
    pub action: Action,
    /// When a roomy secondary disk exists: where this could live instead.
    #[serde(default)]
    pub relocate_to: Option<PathBuf>,
    /// Set when a guard (running process, dirty git tree...) blocks reclaiming.
    pub protected: Option<String>,
    pub preselected: bool,
}

impl Finding {
    pub fn age_days(&self, now: i64) -> Option<f64> {
        self.last_activity.map(|t| (now - t).max(0) as f64 / 86_400.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub device: String,
    pub label: Option<String>,
    pub fs: Option<String>,
    pub mount: Option<PathBuf>,
    pub total: u64,
    pub used: u64,
    pub available: u64,
    /// true = spinning disk.
    pub rotational: bool,
    pub removable: bool,
    pub uuid: Option<String>,
    /// Listed in /etc/fstab (mounted at boot).
    pub in_fstab: bool,
}

impl DiskInfo {
    pub fn kind(&self) -> &'static str {
        if self.rotational {
            "HDD"
        } else if self.device.contains("nvme") {
            "NVMe SSD"
        } else {
            "SSD"
        }
    }
    /// Same convention as `df`: reserved blocks are not counted as free.
    pub fn used_ratio(&self) -> f64 {
        let denom = self.used + self.available;
        if denom == 0 {
            0.0
        } else {
            self.used as f64 / denom as f64
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryTotal {
    pub category: Category,
    pub label: String,
    pub bytes: u64,
    pub reclaimable: u64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub generated_at: i64,
    pub host: String,
    pub duration_ms: u64,
    pub disks: Vec<DiskInfo>,
    pub findings: Vec<Finding>,
    pub totals: Vec<CategoryTotal>,
    pub agents: Vec<crate::agents::AgentActivity>,
    pub growth: Option<crate::history::Growth>,
    pub insights: Vec<Insight>,
    /// `complete` or `partial` (unreadable dirs were skipped).
    pub scan_status: String,
    pub unreadable: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warn,
    Critical,
}

pub fn stable_id(rule: &str, path: &std::path::Path) -> String {
    // FNV-1a: tiny, deterministic across runs and platforms.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in rule.bytes().chain([0u8]).chain(path.as_os_str().as_encoded_bytes().iter().copied()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("f{:012x}", h & 0xffff_ffff_ffff)
}

pub fn human_bytes(b: u64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", U[i])
    } else {
        format!("{v:.1} {}", U[i])
    }
}
