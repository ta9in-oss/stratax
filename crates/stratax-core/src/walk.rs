//! Parallel, single-filesystem directory walker.
//!
//! Sizes are *allocated* bytes (`st_blocks * 512`, what `du` reports), hardlinks are
//! counted once, and the walk never crosses into another mount.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirNode {
    pub name: String,
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub newest: i64,
    /// Kept children, largest first. Small ones are folded into `other_bytes`.
    pub children: Vec<DirNode>,
    pub other_bytes: u64,
    /// Largest regular files directly inside this dir.
    pub top_files: Vec<(String, u64)>,
}

impl DirNode {
    pub fn find(&self, rel: &Path) -> Option<&DirNode> {
        let mut cur = self;
        for comp in rel.components() {
            let name = comp.as_os_str().to_string_lossy();
            cur = cur.children.iter().find(|c| c.name == name)?;
        }
        Some(cur)
    }
}

/// Live counters a UI can poll while a walk runs.
#[derive(Debug, Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub unreadable: AtomicU64,
    pub cancel: AtomicBool,
}

#[derive(Clone)]
pub struct WalkOptions {
    /// Keep tree nodes down to this depth (0 = totals only).
    pub keep_depth: usize,
    /// Fold child dirs smaller than this into `other_bytes`.
    pub min_keep: u64,
    pub top_files: usize,
    pub one_filesystem: bool,
    pub progress: Arc<Progress>,
    pub skip: Arc<Vec<PathBuf>>,
}

static GLOBAL: std::sync::RwLock<Option<Arc<Progress>>> = std::sync::RwLock::new(None);

/// Route counters of every default walk (e.g. per-finding `usage`) into `p`.
pub fn set_global_progress(p: Option<Arc<Progress>>) {
    *GLOBAL.write().unwrap() = p;
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            keep_depth: 0,
            min_keep: 1 << 20,
            top_files: 0,
            one_filesystem: true,
            progress: GLOBAL.read().unwrap().clone().unwrap_or_default(),
            skip: Arc::new(Vec::new()),
        }
    }
}

struct Ctx {
    opts: WalkOptions,
    dev: u64,
    seen: Vec<Mutex<HashSet<(u64, u64)>>>,
}

impl Ctx {
    /// true the first time an inode with nlink > 1 is seen.
    fn first_link(&self, dev: u64, ino: u64) -> bool {
        let shard = (ino as usize) % self.seen.len();
        self.seen[shard].lock().unwrap().insert((dev, ino))
    }
}

pub fn walk(root: &Path, opts: WalkOptions) -> DirNode {
    let dev = fs::symlink_metadata(root).map(|m| m.dev()).unwrap_or(0);
    let ctx = Ctx {
        opts,
        dev,
        seen: (0..64).map(|_| Mutex::new(HashSet::new())).collect(),
    };
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let mut node = walk_dir(&ctx, root, 0);
    node.name = name;
    node
}

/// Convenience: allocated bytes, file count and newest mtime of a path.
pub fn usage(path: &Path) -> DirNode {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_dir() => DirNode {
            name: path.display().to_string(),
            bytes: m.blocks() * 512,
            files: 1,
            newest: m.mtime(),
            ..Default::default()
        },
        Ok(_) => walk(path, WalkOptions::default()),
        Err(_) => DirNode::default(),
    }
}

fn walk_dir(ctx: &Ctx, dir: &Path, depth: usize) -> DirNode {
    let p = &ctx.opts.progress;
    let mut node = DirNode::default();
    if p.cancel.load(Ordering::Relaxed) {
        return node;
    }
    if let Ok(m) = fs::symlink_metadata(dir) {
        node.bytes = m.blocks() * 512;
        node.newest = m.mtime();
    }
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            // Vanishing dirs (agents and mutation runs churn temp trees) are not "unreadable".
            if e.kind() != std::io::ErrorKind::NotFound {
                p.unreadable.fetch_add(1, Ordering::Relaxed);
            }
            return node;
        }
    };
    let mut subdirs: Vec<(PathBuf, String)> = Vec::new();
    let mut files: Vec<(String, u64)> = Vec::new();
    let (mut fbytes, mut fcount) = (0u64, 0u64);
    for entry in rd.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let path = entry.path();
            if ctx.opts.skip.iter().any(|s| s == &path) {
                continue;
            }
            subdirs.push((path, entry.file_name().to_string_lossy().into_owned()));
            continue;
        }
        // DirEntry::metadata does not follow symlinks on unix.
        let Ok(m) = entry.metadata() else { continue };
        if m.nlink() > 1 && !ctx.first_link(m.dev(), m.ino()) {
            continue;
        }
        let b = m.blocks() * 512;
        fbytes += b;
        fcount += 1;
        node.newest = node.newest.max(m.mtime());
        if ctx.opts.top_files > 0 && ft.is_file() {
            files.push((entry.file_name().to_string_lossy().into_owned(), b));
        }
    }
    p.files.fetch_add(fcount, Ordering::Relaxed);
    p.bytes.fetch_add(fbytes, Ordering::Relaxed);
    node.bytes += fbytes;
    node.files = fcount;

    let children: Vec<(String, DirNode)> = subdirs
        .into_par_iter()
        .filter_map(|(path, name)| {
            if ctx.opts.one_filesystem {
                match fs::symlink_metadata(&path) {
                    Ok(m) if m.dev() == ctx.dev => {}
                    _ => return None,
                }
            }
            Some((name, walk_dir(ctx, &path, depth + 1)))
        })
        .collect();

    let keep = depth < ctx.opts.keep_depth;
    for (name, mut child) in children {
        node.bytes += child.bytes;
        node.files += child.files;
        node.dirs += 1 + child.dirs;
        node.newest = node.newest.max(child.newest);
        if keep && child.bytes >= ctx.opts.min_keep {
            child.name = name;
            node.children.push(child);
        } else {
            node.other_bytes += child.bytes;
        }
    }
    node.children
        .sort_unstable_by_key(|c| std::cmp::Reverse(c.bytes));
    if keep && ctx.opts.top_files > 0 {
        files.sort_unstable_by_key(|f| std::cmp::Reverse(f.1));
        files.truncate(ctx.opts.top_files);
        node.top_files = files;
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_hardlinks_once_and_builds_tree() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        fs::create_dir_all(a.join("deep")).unwrap();
        fs::write(a.join("deep/big.bin"), vec![7u8; 256 * 1024]).unwrap();
        fs::write(a.join("deep/linked.bin"), vec![7u8; 8 * 1024]).unwrap();
        fs::hard_link(a.join("deep/linked.bin"), t.path().join("link.bin")).unwrap();
        let opts = WalkOptions {
            keep_depth: 3,
            min_keep: 0,
            top_files: 4,
            ..Default::default()
        };
        let n = walk(t.path(), opts);
        assert_eq!(n.files, 2, "hardlink counted once");
        assert!(n.bytes >= 256 * 1024);
        let deep = n.find(Path::new("a/deep")).expect("tree kept");
        assert_eq!(deep.top_files[0].0, "big.bin");
    }
}
