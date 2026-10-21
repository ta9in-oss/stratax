//! Recognises coding agents and the heavy tools they drive (mutation testers,
//! compilers, containers) from process names and command lines.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentActivity {
    pub agent: String,
    pub kind: AgentKind,
    pub processes: usize,
    pub threads: u64,
    pub cpu: f32,
    pub memory: u64,
    /// Working directories of the agent's processes (projects it is touching).
    pub cwds: Vec<PathBuf>,
    /// Bytes of findings attributed to this agent.
    pub footprint: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// An LLM coding agent.
    Agent,
    /// Something an agent runs that eats disk / CPU (mutation tests, builds).
    Workload,
    /// Infrastructure (docker, browsers driven by agents).
    Infra,
}

/// (family, kind, matcher on lowercase "comm\0cmdline").
const RULES: &[(&str, AgentKind, &[&str])] = &[
    ("cargo-mutants", AgentKind::Workload, &["cargo-mutants", "cargo\0mutants\0"]),
    ("stryker", AgentKind::Workload, &["stryker"]),
    ("mutmut", AgentKind::Workload, &["mutmut"]),
    ("claude-code", AgentKind::Agent, &["/claude\0", "\0claude\0", "claude-code", "@anthropic-ai/claude"]),
    ("codex", AgentKind::Agent, &["codex"]),
    ("cursor", AgentKind::Agent, &["cursor"]),
    ("gemini-cli", AgentKind::Agent, &["gemini"]),
    ("opencode", AgentKind::Agent, &["opencode"]),
    ("aider", AgentKind::Agent, &["aider"]),
    ("copilot", AgentKind::Agent, &["copilot-language-server", "copilot-agent"]),
    ("rust-build", AgentKind::Workload, &["rustc\0", "cargo\0", "rust-analyzer", "/cargo\0"]),
    ("node-tooling", AgentKind::Workload, &["vitest", "jest", "playwright", "next-server", "vite", "tsserver", "turbo"]),
    ("docker", AgentKind::Infra, &["dockerd", "containerd", "docker-proxy", "buildkitd"]),
    ("browser", AgentKind::Infra, &["chrome", "chromium", "firefox"]),
];

/// `comm` is the kernel short name; `cmd` the argv joined by NUL.
pub fn classify(comm: &str, cmd: &[String]) -> Option<(&'static str, AgentKind)> {
    let mut hay = String::with_capacity(128);
    hay.push('\0');
    hay.push_str(&comm.to_lowercase());
    hay.push('\0');
    for a in cmd.iter().take(6) {
        hay.push_str(&a.to_lowercase());
        hay.push('\0');
    }
    RULES
        .iter()
        .find(|(_, _, pats)| pats.iter().any(|p| hay.contains(p)))
        .map(|(n, k, _)| (*n, *k))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    #[test]
    fn classifies_agents_and_workloads() {
        assert_eq!(classify("claude", &s(&["/home/u/.local/bin/claude"])).unwrap().0, "claude-code");
        assert_eq!(classify("cargo", &s(&["cargo", "mutants", "--jobs", "4"])).unwrap().0, "cargo-mutants");
        assert_eq!(classify("node", &s(&["node", "node_modules/.bin/stryker", "run"])).unwrap().0, "stryker");
        assert_eq!(classify("rustc", &s(&["rustc", "--crate-name", "x"])).unwrap().0, "rust-build");
        assert!(classify("bash", &s(&["bash"])).is_none());
    }
}
