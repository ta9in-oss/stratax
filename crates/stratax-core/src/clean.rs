//! Executes reclaim actions. Every action re-checks the guards right before it
//! runs, refuses paths outside the allowed roots, and is logged.

use crate::guard::Guard;
use crate::model::*;
use crate::platform;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    pub id: String,
    pub title: String,
    pub ok: bool,
    pub freed: u64,
    pub message: String,
    /// A command the user has to run themselves (sudo).
    pub manual: Option<String>,
}

pub fn execute(findings: &[Finding], dry_run: bool) -> Vec<Outcome> {
    let guard = Guard::new(platform::current().as_ref());
    // A finding nested in another selected one is covered by it: skip it so
    // nothing is counted (or deleted) twice.
    let covers = |outer: &Finding, inner: &Finding| {
        outer.id != inner.id && !outer.path.starts_with("docker://") && inner.path.starts_with(&outer.path) && inner.path != outer.path
    };
    findings
        .iter()
        .map(|f| match findings.iter().find(|o| covers(o, f)) {
            Some(o) => Outcome { id: f.id.clone(), title: f.title.clone(), ok: true, freed: 0, message: format!("included in “{}”", o.title), manual: None },
            None => run_one(f, &guard, dry_run),
        })
        .collect()
}

/// Bytes a selection would free, counting nested findings once.
pub fn selection_bytes(sel: &[&Finding]) -> u64 {
    sel.iter()
        .filter(|f| !sel.iter().any(|o| o.id != f.id && !o.path.starts_with("docker://") && f.path.starts_with(&o.path) && f.path != o.path))
        .map(|f| f.bytes)
        .sum()
}

fn covered_path(f: &Finding) -> PathBuf {
    // `<target>/*/incremental` style paths: compare on the concrete prefix.
    f.path.ancestors().find(|p| !p.to_string_lossy().contains('*')).unwrap_or(&f.path).to_path_buf()
}

fn run_one(f: &Finding, guard: &Guard, dry: bool) -> Outcome {
    let mut o = Outcome { id: f.id.clone(), title: f.title.clone(), ok: false, freed: 0, message: String::new(), manual: None };
    if let Some(p) = &f.protected {
        o.message = format!("skipped: {p}");
        return o;
    }
    if !f.path.starts_with("docker://") {
        if let Some(who) = guard.in_use(&covered_path(f)) {
            o.message = format!("skipped: now in use by {who}");
            return o;
        }
    }
    let res = match &f.action {
        Action::ReviewOnly => Err("review-only: nothing automatic".to_string()),
        Action::Delete { paths } => delete(paths, dry).map(|_| {
            // The scan already measured it; re-walking here would re-stat
            // hundreds of GB at full priority just to print a number.
            o.freed = f.bytes;
            let b = f.bytes;
            format!("{} {}", if dry { "would free" } else { "freed" }, human_bytes(b))
        }),
        Action::RunTool { program, args } => {
            let cmd = format!("{program} {}", args.join(" "));
            if program == "sudo" {
                o.manual = Some(cmd.clone());
                Err(format!("needs administrator rights — run: {cmd}"))
            } else if dry {
                o.freed = f.bytes;
                Ok(format!("would run: {cmd}"))
            } else {
                run(program, args).map(|out| {
                    o.freed = f.bytes;
                    format!("ran `{cmd}` {out}")
                })
            }
        }
        Action::WorktreeRemove { repo } => {
            let args = ["-C", &repo.to_string_lossy(), "worktree", "remove", &f.path.to_string_lossy()].map(str::to_string);
            if dry {
                o.freed = f.bytes;
                Ok(format!("would run: git {}", args.join(" ")))
            } else {
                run("git", &args).and_then(|_| run("git", &["-C".into(), repo.to_string_lossy().into(), "worktree".into(), "prune".into()])).map(|_| {
                    o.freed = f.bytes;
                    "worktree removed; branch kept".into()
                })
            }
        }
        Action::Relocate { suggested_root } => relocate(&f.path, suggested_root, dry).map(|m| {
            o.freed = f.bytes;
            m
        }),
    };
    match res {
        Ok(m) => {
            o.ok = true;
            o.message = m;
        }
        Err(e) => o.message = e,
    }
    if !dry {
        log(f, &o);
    }
    o
}

/// Only these trees may ever be deleted from, and never their roots.
fn allowed(p: &Path) -> Result<(), String> {
    let home = platform::home();
    let roots = [home.clone(), PathBuf::from("/tmp"), PathBuf::from("/var/tmp")];
    if !p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(format!("refusing non-canonical path {}", p.display()));
    }
    if !roots.iter().any(|r| p.starts_with(r) && p != r) {
        return Err(format!("refusing path outside home/temp: {}", p.display()));
    }
    let shared = [".cache", ".config", ".local", ".local/share", "workspaces", ".claude", ".codex", "Documents", "Desktop", "Downloads", "Pictures", "Videos", "Music"];
    if shared.iter().any(|s| p == home.join(s)) {
        return Err(format!("refusing top-level folder {}", p.display()));
    }
    if p.join(".git").is_dir() {
        return Err(format!("refusing a git repository root: {}", p.display()));
    }
    Ok(())
}

fn delete(paths: &[PathBuf], dry: bool) -> Result<(), String> {
    for p in paths {
        if std::fs::symlink_metadata(p).is_err() {
            continue; // already gone
        }
        allowed(p)?;
        if !dry {
            let meta = std::fs::symlink_metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let r = if meta.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
            r.map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    Ok(())
}

fn relocate(src: &Path, root: &Path, dry: bool) -> Result<String, String> {
    allowed(src)?;
    let home = platform::home();
    let rel = src.strip_prefix(&home).unwrap_or(src);
    let dest = root.join(rel.to_string_lossy().trim_start_matches('/'));
    if dry {
        return Ok(format!("would move to {} and leave a symlink", dest.display()));
    }
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    run("mv", &[src.to_string_lossy().into(), dest.to_string_lossy().into()])?;
    std::os::unix::fs::symlink(&dest, src).map_err(|e| e.to_string())?;
    Ok(format!("moved to {}", dest.display()))
}

fn run(program: &str, args: &[String]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or("").to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn log(f: &Finding, o: &Outcome) {
    let dir = crate::history::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("operations.log")) {
        let _ = writeln!(fh, "{}", serde_json::json!({"at": crate::resources::now(), "path": f.path, "rule": f.rule, "ok": o.ok, "freed": o.freed, "message": o.message}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_selection_is_counted_once() {
        let t = tempfile::tempdir_in(platform::home().join(".cache")).unwrap();
        let target = t.path().join("target");
        std::fs::create_dir_all(target.join("debug/incremental")).unwrap();
        std::fs::write(target.join("debug/incremental/x"), vec![0u8; 32 * 1024]).unwrap();
        let mk = |id: &str, path: PathBuf, paths: Vec<PathBuf>| Finding {
            id: id.into(), rule: id.into(), title: id.into(), path, category: Category::BuildArtifact, owner: None, safety: Safety::Safe,
            bytes: 4096, files: 0, last_activity: None, reason: String::new(), action: Action::Delete { paths }, protected: None,
            preselected: true, relocate_to: None,
        };
        let outer = mk("target", target.clone(), vec![target.clone()]);
        let inner = mk("inc", target.join("*/incremental"), vec![target.join("debug/incremental")]);
        let out = execute(&[outer, inner], true);
        assert!(out[0].freed > 0);
        assert_eq!(out[1].freed, 0, "{}", out[1].message);
    }
    #[test]
    fn refuses_dangerous_paths() {
        let home = platform::home();
        assert!(allowed(&home).is_err());
        assert!(allowed(&home.join(".cache")).is_err());
        assert!(allowed(Path::new("/usr/lib")).is_err());
        assert!(allowed(&home.join("x/../y")).is_err());
        assert!(allowed(&home.join(".cache/pip")).is_ok());
    }
}
