//! Snapshots between scans, so we can answer "what keeps growing back?" and
//! "when will the disk be full?".

use crate::model::Report;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    at: i64,
    /// mount → used bytes
    disks: HashMap<String, u64>,
    /// path → bytes, for the largest findings
    paths: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Growth {
    pub since: i64,
    pub disks: Vec<DiskGrowth>,
    /// Paths whose size changed most since the reference snapshot.
    pub movers: Vec<Mover>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskGrowth {
    pub mount: String,
    pub delta: i64,
    pub per_day: f64,
    /// Days until full at the current rate (None when shrinking / flat).
    pub days_to_full: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mover {
    pub path: String,
    pub delta: i64,
    pub now: u64,
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::platform::home().join(".local/share"));
    base.join("stratax")
}

fn file() -> PathBuf {
    data_dir().join("history.jsonl")
}

pub fn record_and_compare(r: &Report) -> anyhow::Result<Option<Growth>> {
    let snap = Snapshot {
        at: r.generated_at,
        disks: r
            .disks
            .iter()
            .filter_map(|d| Some((d.mount.as_ref()?.display().to_string(), d.used)))
            .collect(),
        paths: r
            .findings
            .iter()
            .take(400)
            .map(|f| (f.path.display().to_string(), f.bytes))
            .collect(),
    };
    let mut prev = load()?;
    std::fs::create_dir_all(data_dir())?;
    // Keep 30 days; rewrite the file when trimming so it never grows unbounded.
    let cutoff = snap.at - 30 * 86_400;
    if prev.first().is_some_and(|s| s.at < cutoff) {
        prev.retain(|s| s.at >= cutoff);
        let body: String = prev
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .map(|l| l + "\n")
            .collect();
        std::fs::write(file(), body)?;
    }
    let mut fh = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file())?;
    writeln!(fh, "{}", serde_json::to_string(&snap)?)?;
    // Movers: against the newest snapshot ≥ 10 min old (what changed lately).
    // Fill rate: against the oldest one within 7 days, so one burst (a
    // mutation run filling /tmp) does not read as "full in 5 hours".
    let Some(recent) = prev.iter().rev().find(|s| snap.at - s.at >= 600) else {
        return Ok(None);
    };
    let long = prev
        .iter()
        .find(|s| snap.at - s.at <= 7 * 86_400)
        .filter(|s| snap.at - s.at >= 3600);
    let mut g = compare(recent, &snap, r);
    match long {
        Some(base) => g.disks = compare(base, &snap, r).disks,
        None => g.disks.iter_mut().for_each(|d| d.days_to_full = None),
    }
    Ok(Some(g))
}

/// One point per past scan: used bytes per mount. For the trend chart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub at: i64,
    pub disks: HashMap<String, u64>,
}

pub fn series() -> Vec<Point> {
    let mut v: Vec<Point> = load()
        .unwrap_or_default()
        .into_iter()
        .map(|s| Point {
            at: s.at,
            disks: s.disks,
        })
        .collect();
    v.sort_by_key(|p| p.at);
    v
}

fn load() -> anyhow::Result<Vec<Snapshot>> {
    let Ok(f) = std::fs::File::open(file()) else {
        return Ok(vec![]);
    };
    Ok(std::io::BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect())
}

fn compare(base: &Snapshot, now: &Snapshot, r: &Report) -> Growth {
    let days = ((now.at - base.at) as f64 / 86_400.0).max(1e-6);
    let disks = r
        .disks
        .iter()
        .filter_map(|d| {
            let m = d.mount.as_ref()?.display().to_string();
            let before = *base.disks.get(&m)?;
            let delta = d.used as i64 - before as i64;
            let per_day = delta as f64 / days;
            let days_to_full = (per_day > 0.0).then(|| d.available as f64 / per_day);
            Some(DiskGrowth {
                mount: m,
                delta,
                per_day,
                days_to_full,
            })
        })
        .collect();
    let mut movers: Vec<Mover> = now
        .paths
        .iter()
        .map(|(p, &b)| Mover {
            path: p.clone(),
            delta: b as i64 - *base.paths.get(p).unwrap_or(&0) as i64,
            now: b,
        })
        .filter(|m| m.delta.unsigned_abs() > 64 << 20)
        .collect();
    movers.sort_by_key(|m| -m.delta);
    movers.truncate(15);
    Growth {
        since: base.at,
        disks,
        movers,
    }
}
