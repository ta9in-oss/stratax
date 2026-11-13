mod tui;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use stratax_core::{advisor, chat, clean, human_bytes, resources::Sampler, scan, Safety};

#[derive(Parser)]
#[command(name = "stratax", version, about = "Disk + resource analyzer that understands AI coding agents")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interactive terminal UI (default).
    Tui,
    /// Fast targeted scan: agent leftovers, build + mutation artifacts, caches, docker.
    Scan {
        #[arg(long)]
        json: bool,
        /// Project roots to walk (default: $HOME).
        #[arg(long)]
        root: Vec<PathBuf>,
        /// Hide findings smaller than this many MB.
        #[arg(long, default_value_t = 32)]
        min_mb: u64,
        /// Full speed on every core (default runs at idle I/O priority).
        #[arg(long)]
        turbo: bool,
    },
    /// CPU, memory, processes and agent activity.
    Status {
        #[arg(long)]
        json: bool,
        /// Stream NDJSON every N seconds.
        #[arg(long)]
        watch: Option<u64>,
    },
    /// Ask the AI advisor (claude-code | anthropic | agnes | openai | offline).
    Advise {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Talk with the advisor about this machine (streams; Ctrl-D to quit).
    Chat {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Ask one question and exit (otherwise an interactive session).
        message: Option<String>,
    },
    /// Reclaim findings by id, or everything preselected.
    Clean {
        ids: Vec<String>,
        #[arg(long)]
        preselected: bool,
        /// Actually do it (default is a dry run).
        #[arg(long)]
        yes: bool,
    },
    /// Full tree of a filesystem / directory (pass 2).
    Tree {
        path: PathBuf,
        #[arg(long, default_value_t = 2)]
        depth: usize,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Cmd::Tui) {
        Cmd::Tui => tui::run(),
        Cmd::Scan { json, root, min_mb, turbo } => {
            let mut o = scan::ScanOptions { min_bytes: min_mb << 20, gentle: !turbo, ..Default::default() };
            if !root.is_empty() {
                o.roots = root;
            }
            let r = scan::scan(&o);
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
                return Ok(());
            }
            println!("stratax · {} · scanned in {:.1}s · {}", r.host, r.duration_ms as f64 / 1000.0, r.scan_status);
            for d in &r.disks {
                let m = d.mount.as_ref().map(|m| m.display().to_string()).unwrap_or_else(|| "NOT MOUNTED".into());
                println!("  {:<8} {:<9} {:<22} {:>9} / {:>9}  {:>3.0}%", d.name, d.kind(), m, human_bytes(d.used), human_bytes(d.total), d.used_ratio() * 100.0);
            }
            println!("\nInsights");
            for i in &r.insights {
                println!("  [{:?}] {}\n         {}", i.severity, i.title, i.detail);
            }
            println!("\nBy category");
            for t in &r.totals {
                println!("  {:<16} {:>9}  reclaimable {:>9}  ({} items)", t.label, human_bytes(t.bytes), human_bytes(t.reclaimable), t.count);
            }
            println!("\nTop findings");
            for f in r.findings.iter().take(40) {
                let mark = if f.protected.is_some() { "🔒" } else if f.preselected { "✔" } else { " " };
                let s = match f.safety { Safety::Safe => "safe", Safety::Rebuild => "rebuild", Safety::Review => "review" };
                println!("  {mark} {} {:>9} {:<8} {}", f.id, human_bytes(f.bytes), s, f.title);
            }
            Ok(())
        }
        Cmd::Status { json, watch } => {
            let mut s = Sampler::new();
            std::thread::sleep(std::time::Duration::from_millis(400));
            loop {
                let snap = s.sample();
                if json || watch.is_some() {
                    println!("{}", serde_json::to_string(&snap)?);
                } else {
                    println!("CPU {:.0}% ({} cores) · load {:.2} {:.2} {:.2}", snap.cpu_total, snap.cpu_cores.len(), snap.load[0], snap.load[1], snap.load[2]);
                    println!("RAM {} / {} · swap {} / {}", human_bytes(snap.mem_used), human_bytes(snap.mem_total), human_bytes(snap.swap_used), human_bytes(snap.swap_total));
                    println!("{} processes · {} threads · {} zombies", snap.process_count, snap.thread_count, snap.zombies);
                    for a in &snap.agents {
                        println!("  {:<14} {:>3} procs {:>5} thr {:>6.1}% cpu {:>9}", a.agent, a.processes, a.threads, a.cpu, human_bytes(a.memory));
                    }
                }
                match watch {
                    Some(n) => std::thread::sleep(std::time::Duration::from_secs(n.max(1))),
                    None => return Ok(()),
                }
            }
        }
        Cmd::Advise { provider, model, json } => {
            if let Some(p) = provider {
                std::env::set_var("STRATAX_PROVIDER", p);
            }
            if let Some(m) = model {
                std::env::set_var("STRATAX_MODEL", m);
            }
            let r = scan::recent_or_scan(900, &Default::default());
            let a = advisor::advise(&r, &advisor::AdvisorConfig::load())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&a)?);
                return Ok(());
            }
            println!("{}\n\n{}\n", a.provider, a.summary);
            for rec in &a.recommendations {
                let f = r.findings.iter().find(|f| f.id == rec.id).unwrap();
                println!("  P{} {:<8} {:>9}  {}\n             {}", rec.priority, rec.verdict, human_bytes(f.bytes), f.title, rec.rationale);
            }
            println!("\nPrevent it coming back:");
            for p in &a.prevention {
                println!("  • {p}");
            }
            Ok(())
        }
        Cmd::Chat { provider, model, message } => {
            use std::io::{BufRead, Write};
            if let Some(p) = provider {
                std::env::set_var("STRATAX_PROVIDER", p);
            }
            if let Some(m) = model {
                std::env::set_var("STRATAX_MODEL", m);
            }
            let cfg = advisor::AdvisorConfig::load();
            let r = scan::recent_or_scan(3600, &Default::default());
            let mut sampler = Sampler::new();
            std::thread::sleep(std::time::Duration::from_millis(400));
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let mut history: Vec<chat::ChatMsg> = vec![];
            let mut ask = |q: String, history: &mut Vec<chat::ChatMsg>| -> anyhow::Result<()> {
                history.push(chat::ChatMsg { role: "user".into(), content: q });
                let live = chat::live_context(&sampler.sample());
                let reply = chat::chat(&r, Some(&live), history, &cfg, &cancel, &mut |t| {
                    print!("{t}");
                    let _ = std::io::stdout().flush();
                })?;
                println!("\n");
                history.push(chat::ChatMsg { role: "assistant".into(), content: reply });
                Ok(())
            };
            if let Some(q) = message {
                return ask(q, &mut history);
            }
            eprintln!("stratax chat · {:?} · scan from {} · Ctrl-D to quit\n", cfg.provider, r.host);
            let stdin = std::io::stdin();
            loop {
                eprint!("› ");
                let mut line = String::new();
                if stdin.lock().read_line(&mut line)? == 0 {
                    return Ok(());
                }
                if line.trim().is_empty() {
                    continue;
                }
                if let Err(e) = ask(line.trim().to_string(), &mut history) {
                    eprintln!("✗ {e}");
                    history.pop();
                }
            }
        }
        Cmd::Clean { ids, preselected, yes } => {
            let r = scan::recent_or_scan(900, &Default::default());
            let pick: Vec<_> = r.findings.into_iter().filter(|f| ids.contains(&f.id) || (preselected && f.preselected)).collect();
            if pick.is_empty() {
                anyhow::bail!("nothing selected — pass finding ids from `stratax scan` or --preselected");
            }
            let out = clean::execute(&pick, !yes);
            let mut total = 0;
            for o in &out {
                total += o.freed;
                println!("  {} {:<50} {}", if o.ok { "✔" } else { "✗" }, o.title, o.message);
            }
            println!("\n{} {}{}", if yes { "Freed" } else { "Would free" }, human_bytes(total), if yes { "" } else { " — re-run with --yes to do it" });
            Ok(())
        }
        Cmd::Tree { path, depth } => {
            let n = stratax_core::full_tree(&path, Default::default());
            fn show(n: &stratax_core::walk::DirNode, lvl: usize, max: usize) {
                println!("{}{:>9}  {}", "  ".repeat(lvl), human_bytes(n.bytes), n.name);
                if lvl < max {
                    for c in n.children.iter().take(12) {
                        show(c, lvl + 1, max);
                    }
                }
            }
            show(&n, 0, depth);
            Ok(())
        }
    }
}
