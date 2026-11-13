//! Stratax core: disk + resource analysis that understands coding agents.

pub mod advisor;
pub mod agents;
pub mod chat;
pub mod clean;
pub mod guard;
pub mod history;
pub mod insights;
pub mod model;
pub mod platform;
pub mod resources;
pub mod rules;
pub mod scan;
pub mod walk;

pub use model::*;

use std::path::Path;
use std::sync::Arc;

/// Shared low-priority pool for every walk. Each worker drops its CPU and
/// I/O priority (both are per-thread on Linux), so a scan can never freeze
/// the desktop or starve a build.
pub fn gentle_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        rayon::ThreadPoolBuilder::new()
            .num_threads((n / 2).clamp(2, 6))
            .thread_name(|i| format!("stratax-walk-{i}"))
            .start_handler(|_| platform::lower_priority())
            .build()
            .expect("thread pool")
    })
}

/// Pass 2 — full tree of one filesystem for the explorer. Minutes on a big
/// disk; poll `progress` from another thread.
pub fn full_tree(root: &Path, progress: Arc<walk::Progress>) -> walk::DirNode {
    gentle_pool().install(|| {
        walk::walk(
            root,
            walk::WalkOptions {
                keep_depth: 6,
                min_keep: 16 << 20,
                top_files: 12,
                one_filesystem: true,
                progress,
                ..Default::default()
            },
        )
    })
}
