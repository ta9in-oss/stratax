//! Turns raw findings into a handful of headline conclusions. These work
//! offline; the AI advisor builds on top of them.

use crate::model::*;

pub fn derive(r: &Report) -> Vec<Insight> {
    let mut v = Vec::new();
    let sum = |pred: &dyn Fn(&Finding) -> bool| -> (u64, usize) {
        let m: Vec<&Finding> = r.findings.iter().filter(|f| pred(f)).collect();
        (m.iter().map(|f| f.bytes).sum(), m.len())
    };

    for d in r.disks.iter().filter(|d| d.mount.is_some() && d.total > 8 << 30) {
        let ratio = d.used_ratio();
        if ratio >= 0.85 {
            let eta = r
                .growth
                .as_ref()
                .and_then(|g| g.disks.iter().find(|x| Some(x.mount.as_str()) == d.mount.as_ref().and_then(|m| m.to_str())))
                .and_then(|g| g.days_to_full)
                .map(|days| format!(" At the current rate it is full in {days:.1} days."))
                .unwrap_or_default();
            v.push(Insight {
                severity: if ratio >= 0.93 { Severity::Critical } else { Severity::Warn },
                title: format!("{} is {:.0}% full", d.mount.as_ref().unwrap().display(), ratio * 100.0),
                detail: format!("{} free of {} on {} ({}).{eta}", human_bytes(d.available), human_bytes(d.total), d.device, d.kind()),
            });
        }
    }

    for d in r.disks.iter().filter(|d| d.mount.is_none() && d.total > 32 << 30) {
        v.push(Insight {
            severity: Severity::Warn,
            title: format!("{} {} is not mounted", d.kind(), d.label.as_deref().unwrap_or(&d.name)),
            detail: format!(
                "{} ({}, {}) is sitting idle. Mount it and move bulky, rarely-read data there — Docker's data-root, model weights, screencasts, old projects — instead of deleting it and watching it come back.",
                d.device,
                human_bytes(d.total),
                d.fs.as_deref().unwrap_or("?")
            ),
        });
    }

    for d in r.disks.iter().filter(|d| d.mount.is_some() && !d.in_fstab && d.total > 32 << 30) {
        v.push(Insight {
            severity: Severity::Info,
            title: format!("{} {} is mounted, but not at boot", d.kind(), d.label.as_deref().unwrap_or(&d.name)),
            detail: format!(
                "{} is auto-mounted by the desktop on first access and is missing from /etc/fstab. Add `UUID={} {} {} defaults,nofail 0 2` before relocating anything onto it.",
                d.device,
                d.uuid.as_deref().unwrap_or("?"),
                d.mount.as_ref().unwrap().display(),
                d.fs.as_deref().unwrap_or("auto")
            ),
        });
    }
    let reloc: Vec<&Finding> = r.findings.iter().filter(|f| f.relocate_to.is_some()).collect();
    if !reloc.is_empty() {
        let b: u64 = reloc.iter().map(|f| f.bytes).sum();
        v.push(Insight {
            severity: Severity::Info,
            title: format!("{} could move to the HDD instead of being deleted", human_bytes(b)),
            detail: format!("{} — models, toolchains and review-only data you probably want to keep.", reloc.iter().take(4).map(|f| f.title.as_str()).collect::<Vec<_>>().join(", ")),
        });
    }

    let (rust, n_rust) = sum(&|f| f.rule == "rust.target");
    if rust > 10 << 30 {
        let mutated: Vec<String> = r
            .findings
            .iter()
            .filter(|f| f.category == Category::MutationTest)
            .filter_map(|f| f.path.parent().map(|p| p.display().to_string()))
            .collect();
        let (mut_bytes, _) = sum(&|f| f.rule == "rust.target" && f.path.parent().is_some_and(|p| mutated.contains(&p.display().to_string())));
        let tail = if mut_bytes > 0 {
            format!(" {} of that sits in projects that run mutation tests — every mutant triggers a rebuild, so incremental/ grows without bound.", human_bytes(mut_bytes))
        } else {
            String::new()
        };
        v.push(Insight {
            severity: Severity::Warn,
            title: format!("Rust build output: {} in {n_rust} target/ dirs", human_bytes(rust)),
            detail: format!("`cargo clean` in idle projects, or run mutation tests with CARGO_INCREMENTAL=0 and a dedicated CARGO_TARGET_DIR you can wipe after each run.{tail}"),
        });
    }

    let (sess, n_sess) = sum(&|f| f.rule == "claude.tmp-sessions" && f.protected.is_none());
    if sess > 1 << 30 {
        v.push(Insight {
            severity: Severity::Warn,
            title: format!("{} in {n_sess} finished agent sessions", human_bytes(sess)),
            detail: "Claude Code scratchpads in the temp dir survive the session. Downloads and builds done by agents end up here.".into(),
        });
    }

    let (docker, _) = sum(&|f| f.category == Category::Container);
    if docker > 5 << 30 {
        v.push(Insight {
            severity: Severity::Warn,
            title: format!("Docker can give back {}", human_bytes(docker)),
            detail: "Build cache and stopped containers come from agents running `docker build` / compose. `docker builder prune -a` is always safe.".into(),
        });
    }

    let live: Vec<&str> = r.agents.iter().filter(|a| a.kind == crate::agents::AgentKind::Agent).map(|a| a.agent.as_str()).collect();
    if !live.is_empty() {
        v.push(Insight {
            severity: Severity::Info,
            title: format!("{} agent process group(s) running: {}", live.len(), live.join(", ")),
            detail: "Anything they hold open or use as a working directory is protected automatically.".into(),
        });
    }

    if let Some(g) = &r.growth {
        if let Some(m) = g.movers.first().filter(|m| m.delta > 1 << 30) {
            v.push(Insight {
                severity: Severity::Info,
                title: format!("Fastest grower: +{}", human_bytes(m.delta as u64)),
                detail: format!("{} since the previous scan.", m.path),
            });
        }
    }
    v
}
