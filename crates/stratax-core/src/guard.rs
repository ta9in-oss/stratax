//! Safety guards: never offer to reclaim something a live process is using,
//! or a worktree with work that exists nowhere else.

use crate::platform::{Platform, ProcPaths};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Guard {
    /// path → "name (pid)" of a process using it (cwd or open fd).
    active: BTreeMap<PathBuf, String>,
    pub procs: Vec<ProcPaths>,
}

impl Guard {
    pub fn new(plat: &dyn Platform) -> Self {
        let procs = plat.process_paths(true);
        let mut active = BTreeMap::new();
        for p in &procs {
            let who = format!("{} (pid {})", p.name, p.pid);
            for path in p.cwd.iter().chain(p.open.iter()) {
                if path != Path::new("/") {
                    active.entry(path.clone()).or_insert_with(|| who.clone());
                }
            }
        }
        Self { active, procs }
    }

    #[cfg(test)]
    pub fn from_paths(paths: &[(&str, &str)]) -> Self {
        Self { active: paths.iter().map(|(p, w)| (PathBuf::from(p), w.to_string())).collect(), procs: vec![] }
    }

    /// Who is using `path` or anything below it. `PathBuf`'s ordering is
    /// component-wise, so every descendant sorts contiguously right after it.
    pub fn in_use(&self, path: &Path) -> Option<&str> {
        let (p, who) = self.active.range(path.to_path_buf()..).next()?;
        p.starts_with(path).then_some(who.as_str())
    }

    pub fn cwds_of<'a>(&'a self, comm: &'a str) -> impl Iterator<Item = &'a Path> + 'a {
        self.procs.iter().filter(move |p| p.name == comm).filter_map(|p| p.cwd.as_deref())
    }

    pub fn is_running(&self, pattern: &str) -> bool {
        self.procs.iter().any(|p| p.name.contains(pattern))
    }
}

/// Main repository of a linked worktree (`.git` is a file pointing into it).
pub fn worktree_repo(wt: &Path) -> Option<PathBuf> {
    let s = std::fs::read_to_string(wt.join(".git")).ok()?;
    let gitdir = PathBuf::from(s.trim().strip_prefix("gitdir:")?.trim());
    // <repo>/.git/worktrees/<name>
    let dotgit = gitdir.parent()?.parent()?;
    dotgit.parent().map(Path::to_path_buf)
}

/// Why a worktree must not be removed, if any reason exists.
pub fn worktree_blocker(wt: &Path) -> Option<String> {
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(wt).args(args).output().ok();
    let Some(st) = git(&["status", "--porcelain", "--untracked-files=normal"]) else {
        return Some("git not available — cannot verify".into());
    };
    if !st.status.success() {
        return Some("not a valid git worktree (git status failed)".into());
    }
    let dirty = String::from_utf8_lossy(&st.stdout).lines().count();
    if dirty > 0 {
        return Some(format!("{dirty} uncommitted/untracked change(s)"));
    }
    let detached = git(&["symbolic-ref", "-q", "HEAD"]).map(|o| !o.status.success()).unwrap_or(true);
    if detached {
        let on_branch = git(&["branch", "-a", "--contains", "HEAD", "--format=%(refname)"])
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);
        if !on_branch {
            return Some("detached HEAD with commits on no branch".into());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descendant_use_protects_ancestor_only() {
        let g = Guard::from_paths(&[("/tmp/a/b/c", "claude (pid 1)")]);
        assert!(g.in_use(Path::new("/tmp/a")).is_some());
        assert!(g.in_use(Path::new("/tmp/a/b/c")).is_some());
        assert!(g.in_use(Path::new("/tmp/ab")).is_none());
        assert!(g.in_use(Path::new("/tmp/a/x")).is_none());
    }
}
