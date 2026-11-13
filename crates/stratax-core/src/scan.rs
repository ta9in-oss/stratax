//! Pass 1 — the fast, targeted scan. Checks every known location, walks
//! project roots for build / mutation artifacts, asks docker & snap, then
//! applies guards and preselection. Typically seconds, not minutes.

use crate::guard::{self, Guard};
use crate::model::*;
use crate::platform::{self, Platform};
use crate::rules::{self, How, ARTIFACTS, KNOWN, WALK_HIDDEN_OK};
use crate::walk::{self, Progress};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct ScanOptions {
    /// Where to look for projects. Defaults to `$HOME`.
    pub roots: Vec<PathBuf>,
    pub max_depth: usize,
    /// Ignore findings smaller than this.
    pub min_bytes: u64,
    pub tools: bool,
    /// Low CPU/I/O priority on half the cores (default). Off = full speed.
    pub gentle: bool,
    pub progress: Arc<Progress>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            roots: vec![platform::home()],
            max_depth: 9,
            min_bytes: 32 << 20,
            tools: true,
            gentle: true,
            progress: Arc::new(Progress::default()),
        }
    }
}

const HOUR: i64 = 3600;
const DAY: i64 = 86_400;

pub fn scan(opts: &ScanOptions) -> Report {
    // Scanning is I/O-bound and runs while agents are busy compiling: never
    // let it compete with them. Half the cores, idle I/O class, nice 10.
    if opts.gentle {
        crate::gentle_pool().install(|| scan_inner(opts))
    } else {
        scan_inner(opts)
    }
}

/// The last report if it is younger than `max_age_secs`, else a fresh scan.
pub fn recent_or_scan(max_age_secs: i64, opts: &ScanOptions) -> Report {
    match load_last() {
        Some(r) if crate::resources::now() - r.generated_at <= max_age_secs => r,
        _ => scan(opts),
    }
}

fn scan_inner(opts: &ScanOptions) -> Report {
    walk::set_global_progress(Some(opts.progress.clone()));
    let r = scan_body(opts);
    walk::set_global_progress(None);
    r
}

fn scan_body(opts: &ScanOptions) -> Report {
    let t0 = Instant::now();
    let plat = platform::current();
    let home = platform::home();
    let now = crate::resources::now();

    let ((disks, guard), (mut findings, tool_findings)) = rayon::join(
        || rayon::join(|| plat.disks(), || Guard::new(plat.as_ref())),
        || {
            rayon::join(
                || {
                    let mut v = known_findings(plat.as_ref(), &home);
                    v.extend(project_artifacts(&opts.roots, opts.max_depth, &home));
                    v.extend(generic_cache(&home));
                    v
                },
                || if opts.tools { tool_findings() } else { vec![] },
            )
        },
    );
    findings.extend(tool_findings);
    findings.retain(|f| f.bytes >= opts.min_bytes);
    dedup_same_path(&mut findings);

    apply_guards(&mut findings, &guard, now);
    for f in findings.iter_mut() {
        f.preselected = preselect(f, now);
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    suggest_relocation(&mut findings, &disks);

    let mut sampler = crate::resources::Sampler::new();
    let mut agents = sampler.sample().agents;
    for a in agents.iter_mut() {
        a.footprint = findings
            .iter()
            .filter(|f| f.owner.as_deref() == Some(a.agent.as_str()))
            .map(|f| f.bytes)
            .sum();
    }
    let totals = category_totals(&findings);
    let unreadable = opts.progress.unreadable.load(Ordering::Relaxed);
    let mut report = Report {
        generated_at: now,
        host: hostname(),
        duration_ms: t0.elapsed().as_millis() as u64,
        disks,
        findings,
        totals,
        agents,
        growth: None,
        insights: vec![],
        scan_status: if unreadable > 0 {
            "partial".into()
        } else {
            "complete".into()
        },
        unreadable,
    };
    report.growth = crate::history::record_and_compare(&report).ok().flatten();
    report.insights = crate::insights::derive(&report);
    save_last(&report);
    report
}

/// The previous report, so UIs can open instantly and rescan in the background.
pub fn load_last() -> Option<Report> {
    serde_json::from_slice(&fs::read(crate::history::data_dir().join("last-report.json")).ok()?)
        .ok()
}

fn save_last(r: &Report) {
    let dir = crate::history::data_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(b) = serde_json::to_vec(r) {
        let tmp = dir.join("last-report.json.tmp");
        if fs::write(&tmp, b).is_ok() {
            let _ = fs::rename(tmp, dir.join("last-report.json"));
        }
    }
}

/// Big, rarely-written data belongs on the slow, roomy disk — not in the bin.
fn suggest_relocation(findings: &mut [Finding], disks: &[DiskInfo]) {
    let home = platform::home();
    let home_dev = disks
        .iter()
        .filter(|d| d.mount.as_ref().is_some_and(|m| home.starts_with(m)))
        .max_by_key(|d| d.mount.as_ref().map(|m| m.as_os_str().len()));
    let Some(home_dev) = home_dev else { return };
    let Some(target) = disks
        .iter()
        // Only disks that mount at boot: otherwise every moved folder becomes a
        // dangling symlink after a reboot until something mounts the disk.
        .filter(|d| {
            d.mount.is_some()
                && d.in_fstab
                && d.device != home_dev.device
                && d.available > 64 << 30
                && !d.removable
        })
        .max_by_key(|d| d.available)
    else {
        return;
    };
    let root = target.mount.clone().unwrap().join("stratax-relocated");
    let mut budget = target.available.saturating_sub(32 << 30);
    for f in findings.iter_mut() {
        let candidate = f.bytes > 1 << 30
            && f.protected.is_none()
            && f.path.starts_with(&home)
            && (matches!(f.category, Category::Models | Category::Toolchain)
                || (f.safety == Safety::Review
                    && matches!(f.category, Category::AppCache | Category::AgentData)));
        if candidate && f.bytes < budget {
            budget -= f.bytes;
            f.relocate_to = Some(root.clone());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn mk(
    rule: &str,
    title: impl Into<String>,
    path: PathBuf,
    category: Category,
    owner: Option<&str>,
    safety: Safety,
    reason: impl Into<String>,
    action: Action,
) -> Finding {
    let u = walk::usage(&path);
    Finding {
        id: stable_id(rule, &path),
        rule: rule.into(),
        title: title.into(),
        category,
        owner: owner.map(str::to_string),
        safety,
        bytes: u.bytes,
        files: u.files,
        last_activity: (u.newest > 0).then_some(u.newest),
        reason: reason.into(),
        action,
        protected: None,
        preselected: false,
        relocate_to: None,
        path,
    }
}

fn known_findings(plat: &dyn Platform, home: &Path) -> Vec<Finding> {
    let uid = platform::uid();
    let tmps = plat.temp_dirs();
    KNOWN
        .par_iter()
        .flat_map_iter(|k| {
            let bases: Vec<PathBuf> = if k.path.starts_with("{tmp}") {
                tmps.iter()
                    .map(|t| rules::expand(k.path, home, t, uid))
                    .collect()
            } else {
                vec![rules::expand(k.path, home, Path::new("/tmp"), uid)]
            };
            bases
                .into_iter()
                .filter(|b| b.exists())
                .flat_map(move |base| expand_known(k, base))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn expand_known(k: &rules::Known, base: PathBuf) -> Vec<Finding> {
    let one = |path: PathBuf, title: String, reason: String, how: How| {
        mk(
            k.id,
            title,
            path.clone(),
            k.category,
            k.owner,
            k.safety,
            reason,
            rules::action_for(how, &path),
        )
    };
    if k.how != How::PerChild {
        return vec![one(base, k.title.into(), k.reason.into(), k.how)];
    }
    let children = |p: &Path| -> Vec<PathBuf> {
        fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match k.id {
        // /tmp/claude-<uid>/<project-slug>/<session-uuid>
        "claude.tmp-sessions" => children(&base)
            .into_par_iter()
            .filter(|p| p.is_dir())
            .flat_map_iter(|proj| {
                let slug = name(&proj);
                let sessions: Vec<PathBuf> =
                    children(&proj).into_iter().filter(|p| p.is_dir()).collect();
                if sessions.is_empty() || !slug.starts_with('-') {
                    return vec![one(
                        proj,
                        format!("Claude Code temp · {slug}"),
                        k.reason.into(),
                        How::Delete,
                    )];
                }
                sessions
                    .into_iter()
                    .map(|s| {
                        let project = slug
                            .trim_start_matches('-')
                            .rsplit('-')
                            .next()
                            .unwrap_or(&slug)
                            .to_string();
                        let sid: String = name(&s).chars().take(8).collect();
                        one(
                            s,
                            format!("Claude session {sid} · {project}"),
                            k.reason.into(),
                            How::Delete,
                        )
                    })
                    .collect()
            })
            .collect(),
        "mutants.tmp" => children(&base)
            .into_par_iter()
            .filter(|p| name(p).starts_with("cargo-mutants"))
            .map(|p| {
                one(
                    p.clone(),
                    format!("cargo-mutants temp · {}", name(&p)),
                    k.reason.into(),
                    How::Delete,
                )
            })
            .collect(),
        "tool.rustup" => {
            let default = Command::new("rustup")
                .args(["default"])
                .output()
                .ok()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string()
                });
            children(&base)
                .into_par_iter()
                .filter(|p| default.as_deref() != Some(name(p).as_str()))
                .map(|p| one(p.clone(), format!("Rust toolchain {}", name(&p)), "Non-default toolchain. `rustup toolchain uninstall` it if no project pins it.".into(), How::Tool("rustup", &[])))
                .map(|mut f| {
                    f.action = Action::RunTool { program: "rustup".into(), args: vec!["toolchain".into(), "uninstall".into(), name(&f.path)] };
                    f
                })
                .collect()
        }
        "tool.pyenv" => {
            let global =
                fs::read_to_string(base.parent().unwrap().join("version")).unwrap_or_default();
            let keep: Vec<String> = global.split_whitespace().map(str::to_string).collect();
            children(&base)
                .into_par_iter()
                .filter(|p| p.is_dir() && !keep.contains(&name(p)))
                .map(|p| one(p.clone(), format!("Python {}", name(&p)), "Not the pyenv global version. Check no project pins it in .python-version.".into(), How::Delete))
                .collect()
        }
        "tool.nvm" => {
            // GUI launches have no NVM_BIN: fall back to the nvm default alias.
            let nvm = base
                .parent()
                .and_then(Path::parent)
                .unwrap_or(&base)
                .to_path_buf();
            let default = fs::read_to_string(nvm.join("alias/default")).ok().map(|v| {
                let v = v.trim().trim_start_matches('v').to_string();
                base.join(format!("v{v}"))
            });
            let current = std::env::var("NVM_BIN")
                .ok()
                .and_then(|b| PathBuf::from(b).parent().map(Path::to_path_buf));
            children(&base)
                .into_par_iter()
                .filter(|p| {
                    current.as_deref() != Some(p.as_path())
                        && default.as_deref() != Some(p.as_path())
                })
                .map(|p| {
                    one(
                        p.clone(),
                        format!("Node {}", name(&p)),
                        "Not the active nvm version.".into(),
                        How::Delete,
                    )
                })
                .collect()
        }
        _ => children(&base)
            .into_par_iter()
            .filter(|p| p.is_dir())
            .map(|p| {
                one(
                    p.clone(),
                    format!("{} · {}", k.title, name(&p)),
                    k.reason.into(),
                    How::Delete,
                )
            })
            .collect(),
    }
}

/// Walk source roots looking for rebuildable artifacts. Never descends into a
/// match, so the cost is proportional to *source* trees, not their output.
fn project_artifacts(roots: &[PathBuf], max_depth: usize, home: &Path) -> Vec<Finding> {
    let mut hits: Vec<(PathBuf, &'static rules::ArtifactRule)> = Vec::new();
    let skip_top: Vec<PathBuf> = [
        ".cache", ".config", ".local", ".cargo", ".rustup", ".npm", ".nvm", ".pyenv", "go", "snap",
        "Android", ".bun", ".vscode", ".m2", ".gradle", ".var", ".claude", ".codex", ".cursor",
        ".ollama",
    ]
    .iter()
    .map(|d| home.join(d))
    .collect();
    let found = std::sync::Mutex::new(Vec::new());
    roots
        .par_iter()
        .for_each(|r| find_artifacts(r, 0, max_depth, &skip_top, &found));
    hits.append(&mut found.into_inner().unwrap());
    hits.into_par_iter()
        .map(|(path, r)| -> Vec<Finding> {
            let mut f = mk(r.id, r.title, path.clone(), r.category, Some(r.owner), r.safety, r.reason, Action::Delete { paths: vec![path.clone()] });
            let project = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
            f.title = format!("{} · {}", r.title, short(&project));
            if r.id == "agent.worktrees" {
                f.title = format!("Agent worktrees · {}", short(&project));
            }
            let mut extra = None;
            if r.id == "rust.target" {
                let (inc, dirs) = rust_incremental(&path);
                if inc > 0 {
                    f.reason = format!("{} incremental/ alone is {}.", r.reason, human_bytes(inc));
                }
                if inc > 256 << 20 {
                    let newest = dirs.iter().map(|d| walk::usage(d).newest).max().unwrap_or(0);
                    let p = path.join("*/incremental");
                    extra = Some(Finding {
                        id: stable_id("rust.incremental", &path),
                        rule: "rust.incremental".into(),
                        title: format!("Rust incremental cache · {}", short(&project)),
                        path: p,
                        category: Category::BuildArtifact,
                        owner: Some("rust".into()),
                        safety: Safety::Safe,
                        bytes: inc,
                        files: 0,
                        last_activity: (newest > 0).then_some(newest),
                        reason: "Incremental-compilation cache only: deleting it never breaks a build, the next one is just slower. Mutation runs and agents rebuilding in parallel make it grow without bound.".into(),
                        action: Action::Delete { paths: dirs },
                        protected: None,
                        preselected: false,
                        relocate_to: None,
                    });
                }
            }
            std::iter::once(f).chain(extra).collect()
        })
        .flatten_iter()
        .collect()
}

fn find_artifacts(
    dir: &Path,
    depth: usize,
    max: usize,
    skip: &[PathBuf],
    out: &std::sync::Mutex<Vec<(PathBuf, &'static rules::ArtifactRule)>>,
) {
    if depth > max || skip.iter().any(|s| s == dir) {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let entries: Vec<_> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .collect();
    let parent_is_claude = dir.file_name().is_some_and(|n| n == ".claude");
    let mut recurse = Vec::new();
    for e in entries {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let path = e.path();
        let rule = ARTIFACTS.iter().find(|r| {
            r.dir == name
                && (r.id != "agent.worktrees" || parent_is_claude)
                && (r.markers.is_empty() || r.markers.iter().any(|m| dir.join(m).exists()))
        });
        if let Some(r) = rule {
            out.lock().unwrap().push((path.clone(), r));
            if r.id != "agent.worktrees" {
                continue;
            }
            // Agent worktrees are full checkouts: look inside for their own target/ etc.
            for wt in fs::read_dir(&path).into_iter().flatten().flatten() {
                recurse.push(wt.path());
            }
            continue;
        }
        if name == ".git"
            || name == "node_modules"
            || (name.starts_with('.') && !WALK_HIDDEN_OK.contains(&name.as_ref()))
        {
            continue;
        }
        recurse.push(path);
    }
    recurse
        .par_iter()
        .for_each(|p| find_artifacts(p, depth + 1, max, skip, out));
}

fn rust_incremental(target: &Path) -> (u64, Vec<PathBuf>) {
    let dirs: Vec<PathBuf> = fs::read_dir(target)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("incremental"))
        .filter(|p| p.is_dir())
        .collect();
    (dirs.iter().map(|p| walk::usage(p).bytes).sum(), dirs)
}

/// Every big `~/.cache/*` child that no specific rule explains.
fn generic_cache(home: &Path) -> Vec<Finding> {
    let covered = rules::covered_cache_children();
    let base = home.join(".cache");
    fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !covered.contains(&e.file_name().to_string_lossy().as_ref()))
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|e| {
            let p = e.path();
            let n = e.file_name().to_string_lossy().into_owned();
            mk("cache.generic", format!("~/.cache/{n}"), p.clone(), Category::AppCache, None, Safety::Rebuild,
                "XDG cache directory that no specific rule knows. Apps must tolerate it disappearing, but some keep expensive-to-rebuild data here (embeddings, indexes) — check before deleting large ones.",
                Action::Delete { paths: vec![p] })
        })
        .collect()
}

fn tool_findings() -> Vec<Finding> {
    let mut v = docker_findings();
    v.extend(snap_findings());
    v.extend(apt_cache());
    v
}

fn docker_findings() -> Vec<Finding> {
    let Ok(out) = Command::new("docker")
        .args(["system", "df", "--format", "{{json .}}"])
        .output()
    else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            let ty = v["Type"].as_str()?.to_string();
            let reclaim = parse_size(v["Reclaimable"].as_str()?.split_whitespace().next()?)?;
            let total = parse_size(v["Size"].as_str()?)?;
            let (rule, args, safety, why): (&str, &[&str], Safety, &str) = match ty.as_str() {
                "Images" => ("docker.images", &["image", "prune", "-a", "-f"], Safety::Rebuild, "Images not used by any container; pulled again on demand."),
                "Containers" => ("docker.containers", &["container", "prune", "-f"], Safety::Rebuild, "Stopped containers and their writable layers."),
                "Local Volumes" => ("docker.volumes", &["volume", "prune", "-f"], Safety::Review, "Volumes no container references — may hold databases from old dev stacks. The action prunes anonymous volumes only; `docker volume prune -a` also removes named ones, check them first."),
                "Build Cache" => ("docker.build-cache", &["builder", "prune", "-a", "-f"], Safety::Safe, "BuildKit layer cache; agents running `docker build` grow this fast."),
                _ => return None,
            };
            let path = PathBuf::from(format!("docker://{}", ty.to_lowercase().replace(' ', "-")));
            Some(Finding {
                id: stable_id(rule, &path),
                rule: rule.into(),
                title: format!("Docker {} ({} total)", ty.to_lowercase(), human_bytes(total)),
                path,
                category: Category::Container,
                owner: Some("docker".into()),
                safety,
                bytes: reclaim,
                files: v["TotalCount"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
                last_activity: None,
                reason: why.into(),
                action: Action::RunTool { program: "docker".into(), args: args.iter().map(|s| s.to_string()).collect() },
                protected: None,
                preselected: false,
                relocate_to: None,
            })
        })
        .collect()
}

fn snap_findings() -> Vec<Finding> {
    let Ok(out) = Command::new("snap").args(["list", "--all"]).output() else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter(|l| l.contains("disabled"))
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            let (name, rev) = (c.first()?, c.get(2)?);
            let path = PathBuf::from(format!("/var/lib/snapd/snaps/{name}_{rev}.snap"));
            let mut f = mk(
                "snap.disabled",
                format!("Old snap {name} r{rev}"),
                path,
                Category::System,
                Some("snap"),
                Safety::Safe,
                "Disabled snap revision kept for rollback.",
                Action::RunTool {
                    program: "sudo".into(),
                    args: vec![
                        "snap".into(),
                        "remove".into(),
                        name.to_string(),
                        format!("--revision={rev}"),
                    ],
                },
            );
            f.last_activity = None;
            Some(f)
        })
        .collect()
}

fn apt_cache() -> Vec<Finding> {
    let p = PathBuf::from("/var/cache/apt/archives");
    if !p.exists() {
        return vec![];
    }
    vec![mk(
        "apt.archives",
        "APT package downloads",
        p,
        Category::System,
        Some("apt"),
        Safety::Safe,
        "Downloaded .deb files.",
        Action::RunTool {
            program: "sudo".into(),
            args: vec!["apt-get".into(), "clean".into()],
        },
    )]
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let idx = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (n, u) = s.split_at(idx);
    let n: f64 = n.trim().parse().ok()?;
    let m = match u.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "KB" | "K" | "KIB" => 1e3,
        "MB" | "M" | "MIB" => 1e6,
        "GB" | "G" | "GIB" => 1e9,
        "TB" | "T" | "TIB" => 1e12,
        _ => return None,
    };
    Some((n * m) as u64)
}

fn dedup_same_path(v: &mut Vec<Finding>) {
    v.sort_by(|a, b| a.path.cmp(&b.path));
    v.dedup_by(|a, b| a.path == b.path);
}

fn apply_guards(findings: &mut [Finding], guard: &Guard, now: i64) {
    // Projects that have a live Claude Code process, as temp-dir slugs.
    let live_slugs: Vec<String> = guard.cwds_of("claude").map(rules::claude_slug).collect();
    let mutants_running = guard.is_running("cargo-mutants");
    findings.par_iter_mut().for_each(|f| {
        if f.path.starts_with("docker://") {
            return;
        }
        let probe = if f.rule == "rust.incremental" {
            f.path
                .parent()
                .and_then(Path::parent)
                .unwrap_or(&f.path)
                .to_path_buf()
        } else {
            f.path.clone()
        };
        if let Some(who) = guard.in_use(&probe) {
            f.protected = Some(format!("in use by {who}"));
            return;
        }
        let age = f.last_activity.map(|t| now - t).unwrap_or(i64::MAX);
        match f.rule.as_str() {
            "claude.tmp-sessions" => {
                let slug = f
                    .path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if live_slugs.contains(&slug) && age < 12 * HOUR {
                    f.protected = Some(
                        "Claude Code is running in this project and the session is recent".into(),
                    );
                } else if age < 2 * HOUR {
                    f.protected = Some("session touched in the last 2 hours".into());
                }
            }
            "mutants.tmp" if mutants_running && age < HOUR => {
                f.protected = Some("a cargo-mutants run is in progress".into())
            }
            "agent.worktrees" | "codex.worktrees" | "cursor.worktrees" => worktree_guard(f),
            _ => {}
        }
    });
}

fn worktree_guard(f: &mut Finding) {
    // A container of worktrees ("<repo>/.claude/worktrees") or a single one.
    let wts: Vec<PathBuf> = if f.path.join(".git").exists() {
        vec![f.path.clone()]
    } else {
        fs::read_dir(&f.path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(".git").exists())
            .collect()
    };
    let blockers: Vec<String> = wts
        .iter()
        .filter_map(|w| {
            guard::worktree_blocker(w).map(|b| {
                format!(
                    "{}: {b}",
                    w.file_name().unwrap_or_default().to_string_lossy()
                )
            })
        })
        .collect();
    if !blockers.is_empty() {
        f.protected = Some(blockers.join("; "));
    } else if let Some(repo) = wts.first().and_then(|w| guard::worktree_repo(w)) {
        if wts.len() == 1 {
            f.action = Action::WorktreeRemove { repo };
        }
    }
}

/// SpaceGuard-style defaults: download caches always; safe items idle ≥ 2
/// days; rebuildable ones idle ≥ 3 weeks; review items and protected ones never.
pub fn preselect(f: &Finding, now: i64) -> bool {
    if f.protected.is_some() || matches!(f.action, Action::ReviewOnly) {
        return false;
    }
    // Toolchains break every project at once when guessed wrong; unknown big
    // caches may hold embeddings or indexes. Both are opt-in only.
    if f.category == Category::Toolchain || (f.rule == "cache.generic" && f.bytes > 1 << 30) {
        return false;
    }
    let age = f.last_activity.map(|t| now - t).unwrap_or(i64::MAX);
    match f.safety {
        Safety::Review => false,
        Safety::Safe if f.category == Category::PackageCache => true,
        Safety::Safe => age >= 2 * DAY,
        Safety::Rebuild => age >= 21 * DAY,
    }
}

/// Totals that never double count nested findings (e.g. a target/ inside a worktree).
pub fn category_totals(findings: &[Finding]) -> Vec<CategoryTotal> {
    let mut sorted: Vec<&Finding> = findings.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut top: Vec<&Finding> = Vec::new();
    for f in sorted {
        if top.last().is_some_and(|p: &&Finding| {
            f.path.starts_with(&p.path) && !f.path.starts_with("docker://")
        }) {
            continue;
        }
        top.push(f);
    }
    let mut map: std::collections::HashMap<Category, CategoryTotal> = Default::default();
    for f in top {
        let e = map.entry(f.category).or_insert_with(|| CategoryTotal {
            category: f.category,
            label: f.category.label().into(),
            bytes: 0,
            reclaimable: 0,
            count: 0,
        });
        e.bytes += f.bytes;
        e.count += 1;
        if f.protected.is_none() && f.safety != Safety::Review {
            e.reclaimable += f.bytes;
        }
    }
    let mut v: Vec<_> = map.into_values().collect();
    v.sort_by_key(|f| std::cmp::Reverse(f.bytes));
    v
}

fn short(p: &str) -> String {
    let home = platform::home().display().to_string();
    p.strip_prefix(&home)
        .map(|r| format!("~{r}"))
        .unwrap_or_else(|| p.to_string())
}

pub fn hostname() -> String {
    fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(sysinfo::System::host_name)
        .unwrap_or_else(|| "localhost".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mutation_and_build_artifacts_without_descending() {
        let t = tempfile::tempdir().unwrap();
        let proj = t.path().join("proj");
        fs::create_dir_all(proj.join("target/debug/incremental/x")).unwrap();
        fs::write(proj.join("Cargo.toml"), "[package]").unwrap();
        fs::write(
            proj.join("target/debug/incremental/x/blob"),
            vec![1u8; 64 * 1024],
        )
        .unwrap();
        fs::create_dir_all(proj.join("mutants.out")).unwrap();
        fs::create_dir_all(proj.join("target/nested/node_modules")).unwrap();
        fs::write(proj.join("target/nested/package.json"), "{}").unwrap();
        let v = project_artifacts(&[t.path().to_path_buf()], 6, Path::new("/nonexistent"));
        let rules: Vec<&str> = v.iter().map(|f| f.rule.as_str()).collect();
        assert!(rules.contains(&"rust.target"));
        assert!(rules.contains(&"mutants.out"));
        assert!(
            !rules.contains(&"node.modules"),
            "must not descend into target/"
        );
        let target = v.iter().find(|f| f.rule == "rust.target").unwrap();
        assert!(target.reason.contains("incremental"));
    }

    #[test]
    fn totals_skip_nested_findings() {
        let f = |p: &str, b: u64| Finding {
            id: p.into(),
            rule: "r".into(),
            title: p.into(),
            path: p.into(),
            category: Category::BuildArtifact,
            owner: None,
            safety: Safety::Rebuild,
            bytes: b,
            files: 0,
            last_activity: None,
            reason: String::new(),
            action: Action::ReviewOnly,
            protected: None,
            preselected: false,
            relocate_to: None,
        };
        let t = category_totals(&[f("/a", 100), f("/a/target", 60), f("/b", 5)]);
        assert_eq!(t[0].bytes, 105);
    }

    #[test]
    fn preselection_follows_age_and_safety() {
        let now = 100 * DAY;
        let mut f = Finding {
            id: "x".into(),
            rule: "r".into(),
            title: "t".into(),
            path: "/x".into(),
            category: Category::BuildArtifact,
            owner: None,
            safety: Safety::Rebuild,
            bytes: 1,
            files: 0,
            last_activity: Some(now - 5 * DAY),
            reason: String::new(),
            action: Action::Delete { paths: vec![] },
            protected: None,
            preselected: false,
            relocate_to: None,
        };
        assert!(!preselect(&f, now));
        f.last_activity = Some(now - 30 * DAY);
        assert!(preselect(&f, now));
        f.protected = Some("busy".into());
        assert!(!preselect(&f, now));
    }

    #[test]
    fn live_claude_project_protects_its_sessions() {
        let now = 1_000_000;
        let cwd = Path::new("/home/u/work/my_app");
        let slug = rules::claude_slug(cwd);
        assert_eq!(slug, "-home-u-work-my-app");
        let guard = Guard::with_procs(vec![crate::platform::ProcPaths {
            pid: 7,
            name: "claude".into(),
            cwd: Some(cwd.into()),
            open: vec![],
        }]);
        let f = |dir: &str, age: i64| Finding {
            id: dir.into(),
            rule: "claude.tmp-sessions".into(),
            title: "s".into(),
            path: PathBuf::from(format!("/tmp/claude-1000/{dir}/abc")),
            category: Category::AgentSession,
            owner: None,
            safety: Safety::Safe,
            bytes: 1,
            files: 0,
            last_activity: Some(now - age),
            reason: String::new(),
            action: Action::Delete { paths: vec![] },
            protected: None,
            preselected: false,
            relocate_to: None,
        };
        let mut v = vec![
            f(&slug, 5 * HOUR),
            f("-home-u-other", 5 * HOUR),
            f("-home-u-other", 10 * 60),
        ];
        apply_guards(&mut v, &guard, now);
        assert!(v[0].protected.is_some(), "live project session");
        assert!(v[1].protected.is_none(), "dead project, 5h idle");
        assert!(v[2].protected.is_some(), "touched 10 minutes ago");
    }

    #[test]
    fn toolchains_and_big_unknown_caches_are_opt_in() {
        let now = 100 * DAY;
        let mut f = Finding {
            id: "x".into(),
            rule: "tool.pyenv".into(),
            title: "t".into(),
            path: "/x".into(),
            category: Category::Toolchain,
            owner: None,
            safety: Safety::Rebuild,
            bytes: 5 << 30,
            files: 0,
            last_activity: Some(0),
            reason: String::new(),
            action: Action::Delete { paths: vec![] },
            protected: None,
            preselected: false,
            relocate_to: None,
        };
        assert!(!preselect(&f, now));
        f.category = Category::AppCache;
        f.rule = "cache.generic".into();
        assert!(!preselect(&f, now));
        f.bytes = 100 << 20;
        assert!(preselect(&f, now));
    }

    #[test]
    fn parses_docker_sizes() {
        assert_eq!(parse_size("23.64GB"), Some(23_640_000_000));
        assert_eq!(parse_size("0B"), Some(0));
    }
}
