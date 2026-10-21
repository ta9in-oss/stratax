//! Terminal UI. Workers run scans, sampling, tree walks and the advisor on
//! background threads and post results over a channel; the UI thread only
//! draws and handles keys, so it never blocks.

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Row, Sparkline, Table, TableState, Wrap},
    DefaultTerminal, Frame,
};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use stratax_core::{
    advisor::{self, Advice, AdvisorConfig, Provider},
    clean::{self, Outcome},
    human_bytes,
    resources::{ResourceSnapshot, Sampler},
    scan, walk, Category, Finding, Report, Safety, Severity,
};

// ── palette ────────────────────────────────────────────────────────────────
const BG: Color = Color::Rgb(14, 16, 20);
const PANEL: Color = Color::Rgb(22, 25, 31);
const LINE: Color = Color::Rgb(44, 49, 59);
const DIM: Color = Color::Rgb(120, 128, 142);
const TEXT: Color = Color::Rgb(220, 224, 230);
const ACCENT: Color = Color::Rgb(94, 234, 212);
const AMBER: Color = Color::Rgb(251, 191, 36);
const ROSE: Color = Color::Rgb(251, 113, 133);
const SKY: Color = Color::Rgb(125, 211, 252);
const VIOLET: Color = Color::Rgb(196, 181, 253);

fn safety_color(s: Safety) -> Color {
    match s {
        Safety::Safe => ACCENT,
        Safety::Rebuild => SKY,
        Safety::Review => AMBER,
    }
}

fn cat_color(c: Category) -> Color {
    match c {
        Category::BuildArtifact => SKY,
        Category::MutationTest => ROSE,
        Category::AgentSession | Category::AgentData | Category::Worktree => VIOLET,
        Category::PackageCache | Category::Toolchain => ACCENT,
        Category::Container => Color::Rgb(96, 165, 250),
        Category::BrowserData | Category::AppCache => Color::Rgb(163, 230, 53),
        Category::Models => AMBER,
        _ => DIM,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Overview,
    Findings,
    Agents,
    Resources,
    Explorer,
    Advisor,
}
const TABS: [(Tab, &str); 6] = [
    (Tab::Overview, "Overview"),
    (Tab::Findings, "Findings"),
    (Tab::Agents, "Agents"),
    (Tab::Resources, "Resources"),
    (Tab::Explorer, "Explorer"),
    (Tab::Advisor, "Advisor"),
];

enum Msg {
    Report(Box<Report>),
    Resources(Box<ResourceSnapshot>),
    Tree(Box<walk::DirNode>),
    Advice(Result<Advice, String>),
    Cleaned(Vec<Outcome>, bool),
}

enum Modal {
    None,
    Detail(String),
    Confirm { outcomes: Vec<Outcome>, ids: Vec<String> },
    Done(Vec<Outcome>),
    Kill(u32, String),
    Help,
}

struct App {
    tab: Tab,
    tx: Sender<Msg>,
    report: Option<Report>,
    scanning: Option<(Arc<walk::Progress>, Instant)>,
    res: Option<ResourceSnapshot>,
    cpu_hist: VecDeque<u64>,
    mem_hist: VecDeque<u64>,
    // findings
    rows: Vec<usize>,
    selected: HashSet<String>,
    fstate: TableState,
    filter: String,
    filtering: bool,
    // resources
    psort: char,
    pstate: TableState,
    agents_only: bool,
    // explorer
    tree: Option<walk::DirNode>,
    tree_progress: Option<Arc<walk::Progress>>,
    tree_path: Vec<String>,
    tstate: TableState,
    // advisor
    advice: Option<Result<Advice, String>>,
    advising: Option<Instant>,
    provider: Provider,
    modal: Modal,
    toast: Option<(String, Instant)>,
    quit: bool,
}

pub fn run() -> anyhow::Result<()> {
    let mut term = ratatui::init();
    let res = App::new().run(&mut term);
    ratatui::restore();
    res
}

impl App {
    fn new() -> Self {
        let (tx, rx) = channel();
        let mut app = App {
            tab: Tab::Overview,
            report: scan::load_last(),
            scanning: None,
            res: None,
            cpu_hist: VecDeque::with_capacity(240),
            mem_hist: VecDeque::with_capacity(240),
            rows: vec![],
            selected: HashSet::new(),
            fstate: TableState::default().with_selected(0),
            filter: String::new(),
            filtering: false,
            psort: 'c',
            pstate: TableState::default().with_selected(0),
            agents_only: false,
            tree: None,
            tree_progress: None,
            tree_path: vec![],
            tstate: TableState::default().with_selected(0),
            advice: None,
            advising: None,
            provider: AdvisorConfig::load().provider,
            modal: Modal::None,
            toast: None,
            quit: false,
            tx,
        };
        app.rx_holder(rx);
        app.preselect_from_report();
        app.start_scan();
        app.start_sampler();
        app
    }

    fn rx_holder(&mut self, rx: Receiver<Msg>) {
        RX.with(|r| *r.borrow_mut() = Some(rx));
    }

    fn run(mut self, term: &mut DefaultTerminal) -> anyhow::Result<()> {
        while !self.quit {
            self.drain();
            term.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(120))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        self.key(k.code, k.modifiers);
                    }
                }
            }
        }
        Ok(())
    }

    fn drain(&mut self) {
        RX.with(|r| {
            let r = r.borrow();
            let Some(rx) = r.as_ref() else { return };
            while let Ok(m) = rx.try_recv() {
                match m {
                    Msg::Report(rep) => {
                        self.report = Some(*rep);
                        self.scanning = None;
                        self.preselect_from_report();
                        self.toast("Scan complete");
                    }
                    Msg::Resources(s) => {
                        push(&mut self.cpu_hist, s.cpu_total.round() as u64);
                        push(&mut self.mem_hist, (s.mem_used * 100 / s.mem_total.max(1)) as u64);
                        self.res = Some(*s);
                    }
                    Msg::Tree(t) => {
                        self.tree = Some(*t);
                        self.tree_progress = None;
                    }
                    Msg::Advice(a) => {
                        self.advice = Some(a);
                        self.advising = None;
                    }
                    Msg::Cleaned(out, dry) => {
                        if dry {
                            let ids = out.iter().filter(|o| o.ok).map(|o| o.id.clone()).collect();
                            self.modal = Modal::Confirm { outcomes: out, ids };
                        } else {
                            self.modal = Modal::Done(out);
                            self.start_scan();
                        }
                    }
                }
            }
        });
        self.rebuild_rows();
    }

    // ── workers ────────────────────────────────────────────────────────────
    fn start_scan(&mut self) {
        if self.scanning.is_some() {
            return;
        }
        let p = Arc::new(walk::Progress::default());
        self.scanning = Some((p.clone(), Instant::now()));
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = scan::scan(&scan::ScanOptions { progress: p, ..Default::default() });
            let _ = tx.send(Msg::Report(Box::new(r)));
        });
    }

    fn start_sampler(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let mut s = Sampler::new();
            loop {
                std::thread::sleep(Duration::from_millis(1200));
                if tx.send(Msg::Resources(Box::new(s.sample()))).is_err() {
                    return;
                }
            }
        });
    }

    fn start_tree(&mut self) {
        if self.tree_progress.is_some() || self.tree.is_some() {
            return;
        }
        let p = Arc::new(walk::Progress::default());
        self.tree_progress = Some(p.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let t = stratax_core::full_tree(&stratax_core::platform::home(), p);
            let _ = tx.send(Msg::Tree(Box::new(t)));
        });
    }

    fn start_advice(&mut self) {
        let Some(r) = self.report.clone() else { return self.toast("Scan first") };
        self.advising = Some(Instant::now());
        let tx = self.tx.clone();
        let mut cfg = AdvisorConfig::load();
        if cfg.provider != self.provider {
            cfg = if self.provider == Provider::OpenAiCompat { AdvisorConfig::agnes() } else { AdvisorConfig { provider: self.provider.clone(), ..Default::default() } };
        }
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Advice(advisor::advise(&r, &cfg).map_err(|e| e.to_string())));
        });
    }

    fn start_clean(&mut self, ids: Vec<String>, dry: bool) {
        let Some(r) = &self.report else { return };
        let pick: Vec<Finding> = r.findings.iter().filter(|f| ids.contains(&f.id)).cloned().collect();
        if pick.is_empty() {
            return self.toast("Nothing selected — space toggles, p selects recommended");
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Cleaned(clean::execute(&pick, dry), dry));
        });
        self.toast(if dry { "Previewing…" } else { "Cleaning…" });
    }

    fn toast(&mut self, s: &str) {
        self.toast = Some((s.to_string(), Instant::now()));
    }

    fn preselect_from_report(&mut self) {
        if let Some(r) = &self.report {
            self.selected = r.findings.iter().filter(|f| f.preselected).map(|f| f.id.clone()).collect();
        }
    }

    fn rebuild_rows(&mut self) {
        let Some(r) = &self.report else { return };
        let q = self.filter.to_lowercase();
        let mut rows: Vec<usize> = (0..r.findings.len())
            .filter(|&i| {
                let f = &r.findings[i];
                q.is_empty()
                    || f.title.to_lowercase().contains(&q)
                    || f.path.to_string_lossy().to_lowercase().contains(&q)
                    || f.owner.as_deref().unwrap_or("").contains(&q)
                    || f.category.label().to_lowercase().contains(&q)
            })
            .collect();
        rows.sort_by(|&a, &b| {
            let (fa, fb) = (&r.findings[a], &r.findings[b]);
            (fa.category.label(), std::cmp::Reverse(fa.bytes)).cmp(&(fb.category.label(), std::cmp::Reverse(fb.bytes)))
        });
        // group order: biggest category first
        let order: Vec<Category> = r.totals.iter().map(|t| t.category).collect();
        rows.sort_by_key(|&i| (order.iter().position(|c| *c == r.findings[i].category).unwrap_or(99), std::cmp::Reverse(r.findings[i].bytes)));
        self.rows = rows;
    }

    // ── keys ───────────────────────────────────────────────────────────────
    fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        if self.filtering {
            match code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.fstate.select(Some(0));
            return;
        }
        match std::mem::replace(&mut self.modal, Modal::None) {
            Modal::None => {}
            Modal::Confirm { ids, outcomes } => {
                if matches!(code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                    self.start_clean(ids, false);
                } else if !matches!(code, KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q')) {
                    self.modal = Modal::Confirm { ids, outcomes };
                }
                return;
            }
            Modal::Kill(pid, name) => {
                match code {
                    KeyCode::Char('y') => self.kill(pid, false, &name),
                    KeyCode::Char('K') => self.kill(pid, true, &name),
                    _ => {}
                }
                return;
            }
            _ => return,
        }
        if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char(c @ '1'..='6') => self.switch(TABS[(c as u8 - b'1') as usize].0),
            KeyCode::Tab => self.switch(TABS[(self.tab_idx() + 1) % TABS.len()].0),
            KeyCode::BackTab => self.switch(TABS[(self.tab_idx() + TABS.len() - 1) % TABS.len()].0),
            KeyCode::Char('r') => self.start_scan(),
            _ => match self.tab {
                Tab::Overview => {
                    if code == KeyCode::Char('c') {
                        self.start_clean(self.selected.iter().cloned().collect(), true);
                    }
                }
                Tab::Findings => self.key_findings(code),
                Tab::Resources => self.key_resources(code),
                Tab::Explorer => self.key_explorer(code),
                Tab::Advisor => match code {
                    KeyCode::Enter | KeyCode::Char('a') => self.start_advice(),
                    KeyCode::Char('p') => {
                        self.provider = match self.provider {
                            Provider::ClaudeCode => Provider::Anthropic,
                            Provider::Anthropic => Provider::OpenAiCompat,
                            Provider::OpenAiCompat => Provider::Offline,
                            Provider::Offline => Provider::ClaudeCode,
                        }
                    }
                    KeyCode::Char('s') => {
                        if let Some(Ok(a)) = &self.advice {
                            self.selected = a.recommendations.iter().filter(|r| r.verdict == "reclaim").map(|r| r.id.clone()).collect();
                            self.switch(Tab::Findings);
                            self.toast("Selected the advisor's reclaim picks");
                        }
                    }
                    _ => {}
                },
                Tab::Agents => {}
            },
        }
    }

    fn tab_idx(&self) -> usize {
        TABS.iter().position(|(t, _)| *t == self.tab).unwrap_or(0)
    }

    fn switch(&mut self, t: Tab) {
        self.tab = t;
        if t == Tab::Explorer {
            self.start_tree();
        }
    }

    fn key_findings(&mut self, code: KeyCode) {
        let n = self.rows.len();
        let cur = self.fstate.selected().unwrap_or(0);
        let cur_id = || self.report.as_ref().and_then(|r| self.rows.get(cur).map(|&i| r.findings[i].id.clone()));
        match code {
            KeyCode::Down | KeyCode::Char('j') => self.fstate.select(Some((cur + 1).min(n.saturating_sub(1)))),
            KeyCode::Up | KeyCode::Char('k') => self.fstate.select(Some(cur.saturating_sub(1))),
            KeyCode::PageDown => self.fstate.select(Some((cur + 15).min(n.saturating_sub(1)))),
            KeyCode::PageUp => self.fstate.select(Some(cur.saturating_sub(15))),
            KeyCode::Home | KeyCode::Char('g') => self.fstate.select(Some(0)),
            KeyCode::End | KeyCode::Char('G') => self.fstate.select(Some(n.saturating_sub(1))),
            KeyCode::Char(' ') => {
                if let Some(id) = cur_id() {
                    let f = self.report.as_ref().unwrap().findings.iter().find(|f| f.id == id).unwrap();
                    if f.protected.is_some() {
                        return self.toast("Protected — see details (enter)");
                    }
                    if !self.selected.remove(&id) {
                        self.selected.insert(id);
                    }
                    self.fstate.select(Some((cur + 1).min(n.saturating_sub(1))));
                }
            }
            KeyCode::Char('p') => self.preselect_from_report(),
            KeyCode::Char('x') => self.selected.clear(),
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('c') | KeyCode::Char('d') => self.start_clean(self.selected.iter().cloned().collect(), true),
            KeyCode::Enter => {
                if let Some(id) = cur_id() {
                    self.modal = Modal::Detail(id);
                }
            }
            _ => {}
        }
    }

    fn sorted_procs(&self) -> Vec<&stratax_core::resources::ProcInfo> {
        let Some(s) = &self.res else { return vec![] };
        let mut v: Vec<_> = s.processes.iter().filter(|p| !self.agents_only || p.agent.is_some()).collect();
        match self.psort {
            'm' => v.sort_by(|a, b| b.memory.cmp(&a.memory)),
            't' => v.sort_by(|a, b| b.threads.cmp(&a.threads)),
            'i' => v.sort_by(|a, b| (b.disk_read + b.disk_write).cmp(&(a.disk_read + a.disk_write))),
            _ => v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
        }
        v
    }

    fn key_resources(&mut self, code: KeyCode) {
        let n = self.sorted_procs().len();
        let cur = self.pstate.selected().unwrap_or(0);
        match code {
            KeyCode::Down | KeyCode::Char('j') => self.pstate.select(Some((cur + 1).min(n.saturating_sub(1)))),
            KeyCode::Up | KeyCode::Char('k') => self.pstate.select(Some(cur.saturating_sub(1))),
            KeyCode::Char(c @ ('c' | 'm' | 't' | 'i')) => self.psort = c,
            KeyCode::Char('a') => self.agents_only = !self.agents_only,
            KeyCode::Char('K') | KeyCode::Delete => {
                if let Some(p) = self.sorted_procs().get(cur) {
                    self.modal = Modal::Kill(p.pid, p.name.clone());
                }
            }
            _ => {}
        }
    }

    fn kill(&mut self, pid: u32, force: bool, name: &str) {
        match stratax_core::resources::signal(pid, force) {
            Ok(()) => self.toast(&format!("Sent {} to {name} ({pid})", if force { "SIGKILL" } else { "SIGTERM" })),
            Err(e) => self.toast(&e.to_string()),
        }
    }

    fn tree_node(&self) -> Option<&walk::DirNode> {
        let mut n = self.tree.as_ref()?;
        for name in &self.tree_path {
            n = n.children.iter().find(|c| &c.name == name)?;
        }
        Some(n)
    }

    fn key_explorer(&mut self, code: KeyCode) {
        let n = self.tree_node().map(|n| n.children.len()).unwrap_or(0);
        let cur = self.tstate.selected().unwrap_or(0);
        match code {
            KeyCode::Down | KeyCode::Char('j') => self.tstate.select(Some((cur + 1).min(n.saturating_sub(1)))),
            KeyCode::Up | KeyCode::Char('k') => self.tstate.select(Some(cur.saturating_sub(1))),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(c) = self.tree_node().and_then(|t| t.children.get(cur)).map(|c| c.name.clone()) {
                    self.tree_path.push(c);
                    self.tstate.select(Some(0));
                }
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                self.tree_path.pop();
                self.tstate.select(Some(0));
            }
            KeyCode::Char('R') => {
                self.tree = None;
                self.tree_path.clear();
                self.start_tree();
            }
            _ => {}
        }
    }

    // ── drawing ────────────────────────────────────────────────────────────
    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::new().style(Style::new().bg(BG).fg(TEXT)), area);
        let [head, body, foot] = Layout::vertical([Constraint::Length(3), Constraint::Min(5), Constraint::Length(1)]).areas(area);
        self.draw_header(f, head);
        let body = body.inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
        match self.tab {
            Tab::Overview => self.draw_overview(f, body),
            Tab::Findings => self.draw_findings(f, body),
            Tab::Agents => self.draw_agents(f, body),
            Tab::Resources => self.draw_resources(f, body),
            Tab::Explorer => self.draw_explorer(f, body),
            Tab::Advisor => self.draw_advisor(f, body),
        }
        self.draw_footer(f, foot);
        self.draw_modal(f, area);
    }

    fn draw_header(&self, f: &mut Frame, a: Rect) {
        let block = Block::new().borders(Borders::BOTTOM).border_style(Style::new().fg(LINE));
        let inner = block.inner(a);
        f.render_widget(block, a);
        let [brand, tabs, status] = Layout::horizontal([Constraint::Length(24), Constraint::Min(10), Constraint::Length(44)]).areas(inner);
        let host = self.report.as_ref().map(|r| r.host.clone()).unwrap_or_default();
        f.render_widget(
            Paragraph::new(Line::from(vec![" ◆ ".fg(ACCENT).bold(), "STRATAX".bold().fg(TEXT), format!("  {host}").fg(DIM)])),
            Rect { y: brand.y + 1, height: 1, ..brand },
        );
        let mut spans = vec![];
        for (i, (t, name)) in TABS.iter().enumerate() {
            let on = *t == self.tab;
            spans.push(Span::styled(format!(" {} ", i + 1), Style::new().fg(if on { BG } else { DIM }).bg(if on { ACCENT } else { BG })));
            spans.push(Span::styled(format!(" {name}  "), if on { Style::new().fg(TEXT).bold() } else { Style::new().fg(DIM) }));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), Rect { y: tabs.y + 1, height: 1, ..tabs });
        let st = if let Some((p, t0)) = &self.scanning {
            let spin = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][(t0.elapsed().as_millis() / 90) as usize % 10];
            Line::from(vec![
                format!("{spin} scanning ").fg(ACCENT),
                format!("{} files · {} · {:.0}s", p.files.load(Ordering::Relaxed), human_bytes(p.bytes.load(Ordering::Relaxed)), t0.elapsed().as_secs_f32()).fg(DIM),
            ])
        } else if let Some(r) = &self.report {
            Line::from(vec!["● ".fg(ACCENT), format!("scanned {} · {:.1}s · {}", ago(r.generated_at), r.duration_ms as f64 / 1000.0, r.scan_status).fg(DIM)])
        } else {
            Line::from("no scan yet".fg(DIM))
        };
        f.render_widget(Paragraph::new(st).right_aligned(), Rect { y: status.y + 1, height: 1, width: status.width.saturating_sub(1), ..status });
    }

    fn draw_footer(&self, f: &mut Frame, a: Rect) {
        if let Some((t, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(4) {
                f.render_widget(Paragraph::new(Line::from(vec![" ▸ ".fg(ACCENT), t.clone().fg(TEXT)])).bg(PANEL), a);
                return;
            }
        }
        let keys: &[(&str, &str)] = match self.tab {
            Tab::Findings => &[("␣", "toggle"), ("p", "recommended"), ("x", "clear"), ("/", "filter"), ("⏎", "details"), ("c", "preview clean")],
            Tab::Resources => &[("c/m/t/i", "sort"), ("a", "agents only"), ("K", "kill"), ("↑↓", "move")],
            Tab::Explorer => &[("⏎/→", "open"), ("⌫/←", "up"), ("R", "rescan")],
            Tab::Advisor => &[("a", "ask"), ("p", "provider"), ("s", "select picks")],
            Tab::Overview => &[("c", "preview clean of recommended")],
            Tab::Agents => &[],
        };
        let mut spans = vec![];
        for (k, d) in keys.iter().chain([("1-6", "views"), ("r", "rescan"), ("?", "help"), ("q", "quit")].iter()) {
            spans.push(Span::styled(format!(" {k} "), Style::new().fg(TEXT).bg(LINE)));
            spans.push(Span::styled(format!(" {d}  "), Style::new().fg(DIM)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), a);
    }

    fn panel(title: &str) -> Block<'static> {
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(LINE))
            .title(Line::from(vec![" ".into(), title.to_uppercase().fg(DIM).add_modifier(Modifier::BOLD), " ".into()]))
            .style(Style::new().bg(PANEL))
    }

    fn draw_overview(&self, f: &mut Frame, a: Rect) {
        let Some(r) = &self.report else {
            f.render_widget(Paragraph::new("Running the first scan…").fg(DIM).centered(), a);
            return;
        };
        let disks: Vec<_> = r.disks.iter().filter(|d| d.total > if d.mount.is_some() { 1 << 30 } else { 16 << 30 }).collect();
        let [disk_a, mid, bottom] = Layout::vertical([Constraint::Length(disks.len() as u16 * 2 + 2), Constraint::Min(8), Constraint::Length(9)]).areas(a);

        // disks
        let blk = Self::panel("Disks");
        let inner = blk.inner(disk_a);
        f.render_widget(blk, disk_a);
        for (i, d) in disks.iter().enumerate() {
            let row = Rect { y: inner.y + i as u16 * 2, height: 1, ..inner };
            let [label, gauge, nums] = Layout::horizontal([Constraint::Length(34), Constraint::Min(10), Constraint::Length(26)]).areas(row);
            let name = d.label.clone().unwrap_or_else(|| d.name.clone());
            let mount = d.mount.as_ref().map(|m| m.display().to_string()).unwrap_or_else(|| "not mounted".into());
            f.render_widget(Paragraph::new(Line::from(vec![format!(" {:<9}", d.kind()).fg(if d.rotational { AMBER } else { SKY }), format!("{name} ").bold(), mount.fg(DIM)])), label);
            if d.mount.is_none() {
                f.render_widget(Paragraph::new(" ◌ idle — mount it and move bulky data here").fg(AMBER), gauge);
            } else {
                let ratio = d.used_ratio();
                let c = if ratio > 0.9 { ROSE } else if ratio > 0.8 { AMBER } else { ACCENT };
                f.render_widget(Gauge::default().ratio(ratio.min(1.0)).gauge_style(Style::new().fg(c).bg(LINE)).label(format!("{:.0}%", ratio * 100.0)).use_unicode(true), gauge);
            }
            let nums_text = match &d.mount {
                Some(_) => format!("{} free of {} ", human_bytes(d.available), human_bytes(d.total)),
                None => format!("{} · {} ", human_bytes(d.total), d.fs.as_deref().unwrap_or("?")),
            };
            f.render_widget(Paragraph::new(nums_text).fg(DIM).right_aligned(), nums);
        }

        // categories + insights
        let [cats, ins] = Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)]).areas(mid);
        let blk = Self::panel("Where the space goes");
        let ci = blk.inner(cats);
        f.render_widget(blk, cats);
        let max = r.totals.first().map(|t| t.bytes).unwrap_or(1).max(1);
        let rows: Vec<Row> = r
            .totals
            .iter()
            .map(|t| {
                let w = 18usize;
                let full = (t.bytes as f64 / max as f64 * w as f64).round() as usize;
                let rec = (t.reclaimable as f64 / max as f64 * w as f64).round() as usize;
                let bar = Line::from(vec![
                    "█".repeat(rec.min(full)).fg(cat_color(t.category)),
                    "▒".repeat(full.saturating_sub(rec)).fg(cat_color(t.category)),
                    "·".repeat(w - full.min(w)).fg(LINE),
                ]);
                Row::new(vec![Line::from(t.label.clone()), bar, Line::from(human_bytes(t.bytes)).right_aligned(), Line::from(human_bytes(t.reclaimable).fg(ACCENT)).right_aligned()])
            })
            .collect();
        f.render_widget(
            Table::new(rows, [Constraint::Length(16), Constraint::Length(22), Constraint::Length(9), Constraint::Length(10)])
                .header(Row::new(vec![Line::from(""), Line::from("█ free-able  ▒ kept"), Line::from("SIZE").right_aligned(), Line::from("FREE-ABLE").right_aligned()]).fg(DIM)),
            ci,
        );

        let blk = Self::panel("Insights");
        let ii = blk.inner(ins);
        f.render_widget(blk, ins);
        let mut lines = vec![];
        for i in &r.insights {
            let (ic, c) = match i.severity {
                Severity::Critical => ("▲", ROSE),
                Severity::Warn => ("●", AMBER),
                Severity::Info => ("○", SKY),
            };
            lines.push(Line::from(vec![format!("{ic} ").fg(c), i.title.clone().bold()]));
            lines.push(Line::from(format!("  {}", i.detail)).fg(DIM));
            lines.push(Line::raw(""));
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), ii);

        // reclaim summary
        let sel: Vec<&Finding> = r.findings.iter().filter(|f| self.selected.contains(&f.id)).collect();
        let sel_bytes: u64 = sel.iter().map(|f| f.bytes).sum();
        let blk = Self::panel("Recommended now");
        let bi = blk.inner(bottom);
        f.render_widget(blk, bottom);
        let [big, list] = Layout::horizontal([Constraint::Length(30), Constraint::Min(10)]).areas(bi);
        f.render_widget(
            Paragraph::new(vec![
                Line::raw(""),
                Line::from(human_bytes(sel_bytes)).fg(ACCENT).bold().centered(),
                Line::from(format!("in {} items", sel.len())).fg(DIM).centered(),
                Line::raw(""),
                Line::from(vec![" c ".fg(BG).bg(ACCENT), " preview cleanup".fg(TEXT)]).centered(),
            ]),
            big,
        );
        let mut top = sel.clone();
        top.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        let lines: Vec<Line> = top.iter().take(bi.height as usize).map(|f| Line::from(vec![format!("{:>9}  ", human_bytes(f.bytes)).fg(TEXT), "■ ".fg(safety_color(f.safety)), f.title.clone().fg(DIM)])).collect();
        f.render_widget(Paragraph::new(lines), list);
    }

    fn draw_findings(&mut self, f: &mut Frame, a: Rect) {
        let Some(r) = &self.report else { return };
        let [bar, table] = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(a);
        let sel_bytes: u64 = r.findings.iter().filter(|f| self.selected.contains(&f.id)).map(|f| f.bytes).sum();
        let filt = if self.filtering || !self.filter.is_empty() { format!("  filter: {}{}", self.filter, if self.filtering { "▏" } else { "" }) } else { String::new() };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                format!(" {} selected · ", self.selected.len()).fg(DIM),
                human_bytes(sel_bytes).fg(ACCENT).bold(),
                format!("   {} findings", self.rows.len()).fg(DIM),
                filt.fg(AMBER),
                "     ■ safe".fg(ACCENT),
                "  ■ rebuild".fg(SKY),
                "  ■ review".fg(AMBER),
                "  🔒 protected".fg(DIM),
            ])),
            bar,
        );
        let max = r.findings.first().map(|f| f.bytes).unwrap_or(1).max(1);
        let now = r.generated_at;
        let mut last_cat = None;
        let rows: Vec<Row> = self
            .rows
            .iter()
            .map(|&i| {
                let fi = &r.findings[i];
                let head = last_cat != Some(fi.category);
                last_cat = Some(fi.category);
                let mark = if fi.protected.is_some() {
                    "🔒".to_string()
                } else if self.selected.contains(&fi.id) {
                    "◉".into()
                } else {
                    "○".into()
                };
                let w = ((fi.bytes as f64 / max as f64).sqrt() * 12.0).round() as usize;
                let cat = if head { fi.category.label().to_string() } else { String::new() };
                Row::new(vec![
                    Line::from(cat).fg(cat_color(fi.category)).bold(),
                    Line::from(mark).fg(if self.selected.contains(&fi.id) { ACCENT } else { DIM }),
                    Line::from(vec!["▍".fg(safety_color(fi.safety)), fi.title.clone().into()]),
                    Line::from(fi.owner.clone().unwrap_or_default()).fg(VIOLET),
                    Line::from(fi.age_days(now).map(|d| idle(d)).unwrap_or_default()).fg(DIM),
                    Line::from(vec!["━".repeat(w.max(1)).fg(cat_color(fi.category))]),
                    Line::from(human_bytes(fi.bytes)).right_aligned(),
                ])
            })
            .collect();
        let t = Table::new(rows, [Constraint::Length(16), Constraint::Length(3), Constraint::Min(30), Constraint::Length(14), Constraint::Length(10), Constraint::Length(13), Constraint::Length(10)])
            .header(Row::new(vec!["CATEGORY", "", "ITEM", "OWNER", "IDLE", "", "SIZE"]).fg(DIM).bottom_margin(0))
            .row_highlight_style(Style::new().bg(LINE))
            .block(Self::panel("Findings"));
        f.render_stateful_widget(t, table, &mut self.fstate);
    }

    fn draw_agents(&self, f: &mut Frame, a: Rect) {
        let Some(r) = &self.report else { return };
        let [top, bottom] = Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(a);
        // footprint by owner
        let mut by: std::collections::BTreeMap<String, (u64, usize, u64)> = Default::default();
        for fi in &r.findings {
            let e = by.entry(fi.owner.clone().unwrap_or_else(|| "other".into())).or_default();
            e.0 += fi.bytes;
            e.1 += 1;
            if fi.protected.is_none() && fi.safety != Safety::Review {
                e.2 += fi.bytes;
            }
        }
        let mut v: Vec<_> = by.into_iter().collect();
        v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        let max = v.first().map(|x| x.1 .0).unwrap_or(1).max(1);
        let rows: Vec<Row> = v
            .iter()
            .map(|(o, (b, n, rec))| {
                let w = (*b as f64 / max as f64 * 30.0).round() as usize;
                let agentish = ["claude-code", "claude-desktop", "codex", "cursor", "cargo-mutants", "stryker", "mutmut"].contains(&o.as_str());
                Row::new(vec![
                    Line::from(o.clone()).fg(if agentish { VIOLET } else { TEXT }).bold(),
                    Line::from("█".repeat(w.max(1))).fg(if agentish { VIOLET } else { SKY }),
                    Line::from(human_bytes(*b)).right_aligned(),
                    Line::from(human_bytes(*rec)).fg(ACCENT).right_aligned(),
                    Line::from(format!("{n}")).fg(DIM).right_aligned(),
                ])
            })
            .collect();
        f.render_widget(
            Table::new(rows, [Constraint::Length(16), Constraint::Length(31), Constraint::Length(10), Constraint::Length(10), Constraint::Length(6)])
                .header(Row::new(vec!["WHO MADE IT", "", "SIZE", "FREE-ABLE", "ITEMS"]).fg(DIM))
                .block(Self::panel("Footprint by agent / tool")),
            top,
        );
        // live
        let live = self.res.as_ref().map(|s| s.agents.clone()).unwrap_or_else(|| r.agents.clone());
        let rows: Vec<Row> = live
            .iter()
            .map(|g| {
                Row::new(vec![
                    Line::from(g.agent.clone()).fg(VIOLET).bold(),
                    Line::from(format!("{}", g.processes)).right_aligned(),
                    Line::from(format!("{}", g.threads)).right_aligned(),
                    Line::from(format!("{:.1}%", g.cpu)).right_aligned(),
                    Line::from(human_bytes(g.memory)).right_aligned(),
                    Line::from(g.cwds.iter().take(3).map(|c| short_home(c)).collect::<Vec<_>>().join("  ")).fg(DIM),
                ])
            })
            .collect();
        f.render_widget(
            Table::new(rows, [Constraint::Length(16), Constraint::Length(6), Constraint::Length(8), Constraint::Length(8), Constraint::Length(10), Constraint::Min(10)])
                .header(Row::new(vec!["RUNNING NOW", "PROCS", "THREADS", "CPU", "MEMORY", "WORKING IN"]).fg(DIM))
                .block(Self::panel("Live agent activity")),
            bottom,
        );
    }

    fn draw_resources(&mut self, f: &mut Frame, a: Rect) {
        let Some(s) = self.res.clone() else {
            f.render_widget(Paragraph::new("Sampling…").fg(DIM).centered(), a);
            return;
        };
        let [top, procs] = Layout::vertical([Constraint::Length(11), Constraint::Min(5)]).areas(a);
        let [cpu, mem] = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(top);

        let blk = Self::panel(&format!("CPU · {}", s.cpu_brand));
        let ci = blk.inner(cpu);
        f.render_widget(blk, cpu);
        let [line, spark, cores] = Layout::vertical([Constraint::Length(1), Constraint::Length(4), Constraint::Min(2)]).areas(ci);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                format!(" {:>5.1}% ", s.cpu_total).fg(ACCENT).bold(),
                format!("load {:.2} {:.2} {:.2} · {} procs · {} threads · {} zombies · up {}", s.load[0], s.load[1], s.load[2], s.process_count, s.thread_count, s.zombies, uptime(s.uptime)).fg(DIM),
            ])),
            line,
        );
        let hist: Vec<u64> = self.cpu_hist.iter().copied().collect();
        f.render_widget(Sparkline::default().data(&hist).max(100).style(Style::new().fg(ACCENT)).bar_set(symbols::bar::NINE_LEVELS), spark);
        let per = (cores.width as usize / s.cpu_cores.len().max(1)).clamp(3, 8);
        let spans: Vec<Span> = s
            .cpu_cores
            .iter()
            .map(|c| {
                let lvl = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"][((c / 100.0) * 7.0).round().clamp(0.0, 7.0) as usize];
                let col = if *c > 85.0 { ROSE } else if *c > 50.0 { AMBER } else { ACCENT };
                Span::styled(format!("{:^w$}", lvl.repeat(per.saturating_sub(1)), w = per), Style::new().fg(col))
            })
            .collect();
        f.render_widget(Paragraph::new(vec![Line::from(spans), Line::from(" per core").fg(DIM)]), cores);

        let blk = Self::panel("Memory");
        let mi = blk.inner(mem);
        f.render_widget(blk, mem);
        let [g1, g2, sp] = Layout::vertical([Constraint::Length(2), Constraint::Length(2), Constraint::Min(2)]).areas(mi);
        let mr = s.mem_used as f64 / s.mem_total.max(1) as f64;
        f.render_widget(
            Gauge::default().ratio(mr.min(1.0)).label(format!("RAM {} / {}", human_bytes(s.mem_used), human_bytes(s.mem_total))).gauge_style(Style::new().fg(if mr > 0.9 { ROSE } else { VIOLET }).bg(LINE)).use_unicode(true),
            Rect { height: 1, ..g1 },
        );
        let sr = s.swap_used as f64 / s.swap_total.max(1) as f64;
        f.render_widget(
            Gauge::default().ratio(sr.min(1.0)).label(format!("swap {} / {}", human_bytes(s.swap_used), human_bytes(s.swap_total))).gauge_style(Style::new().fg(AMBER).bg(LINE)).use_unicode(true),
            Rect { height: 1, ..g2 },
        );
        let mh: Vec<u64> = self.mem_hist.iter().copied().collect();
        f.render_widget(Sparkline::default().data(&mh).max(100).style(Style::new().fg(VIOLET)), sp);

        let list = self.sorted_procs();
        let rows: Vec<Row> = list
            .iter()
            .take(400)
            .map(|p| {
                Row::new(vec![
                    Line::from(format!("{}", p.pid)).fg(DIM),
                    Line::from(p.name.clone()).bold(),
                    Line::from(p.agent.clone().unwrap_or_default()).fg(VIOLET),
                    Line::from(format!("{:.1}", p.cpu)).fg(if p.cpu > 50.0 { AMBER } else { TEXT }).right_aligned(),
                    Line::from(human_bytes(p.memory)).right_aligned(),
                    Line::from(format!("{}", p.threads)).right_aligned(),
                    Line::from(human_bytes(p.disk_read + p.disk_write)).fg(DIM).right_aligned(),
                    Line::from(p.cmd.chars().take(120).collect::<String>()).fg(DIM),
                ])
            })
            .collect();
        let hdr = |k: char, s: &str| if self.psort == k { format!("{s}▾") } else { s.to_string() };
        let title = format!("Processes{}", if self.agents_only { " · agents & workloads" } else { "" });
        f.render_stateful_widget(
            Table::new(rows, [Constraint::Length(8), Constraint::Length(18), Constraint::Length(14), Constraint::Length(7), Constraint::Length(10), Constraint::Length(8), Constraint::Length(10), Constraint::Min(10)])
                .header(Row::new(vec!["PID".into(), "NAME".into(), "AGENT".into(), hdr('c', "CPU%"), hdr('m', "MEM"), hdr('t', "THR"), hdr('i', "I/O"), "COMMAND".into()]).fg(DIM))
                .row_highlight_style(Style::new().bg(LINE))
                .block(Self::panel(&title)),
            procs,
            &mut self.pstate,
        );
    }

    fn draw_explorer(&mut self, f: &mut Frame, a: Rect) {
        if let Some(p) = &self.tree_progress {
            f.render_widget(
                Paragraph::new(vec![
                    Line::raw(""),
                    Line::from("Walking your home directory (full pass, low priority)…").fg(DIM).centered(),
                    Line::from(format!("{} files · {}", p.files.load(Ordering::Relaxed), human_bytes(p.bytes.load(Ordering::Relaxed)))).fg(ACCENT).centered(),
                ])
                .block(Self::panel("Explorer")),
                a,
            );
            return;
        }
        let Some(node) = self.tree_node().cloned() else { return };
        let crumbs = std::iter::once("~".to_string()).chain(self.tree_path.iter().cloned()).collect::<Vec<_>>().join(" › ");
        let [main, side] = Layout::horizontal([Constraint::Percentage(66), Constraint::Percentage(34)]).areas(a);
        let max = node.children.first().map(|c| c.bytes).unwrap_or(1).max(1);
        let rows: Vec<Row> = node
            .children
            .iter()
            .map(|c| {
                let pct = c.bytes as f64 / node.bytes.max(1) as f64;
                let w = (c.bytes as f64 / max as f64 * 24.0).round() as usize;
                Row::new(vec![
                    Line::from(format!("{}{}", c.name, if c.children.is_empty() { "" } else { "/" })).bold(),
                    Line::from(vec!["█".repeat(w.max(1)).fg(if pct > 0.25 { ROSE } else if pct > 0.08 { AMBER } else { SKY }), "".into()]),
                    Line::from(format!("{:.1}%", pct * 100.0)).fg(DIM).right_aligned(),
                    Line::from(human_bytes(c.bytes)).right_aligned(),
                    Line::from(ago(c.newest)).fg(DIM).right_aligned(),
                ])
            })
            .collect();
        f.render_stateful_widget(
            Table::new(rows, [Constraint::Min(20), Constraint::Length(25), Constraint::Length(7), Constraint::Length(10), Constraint::Length(12)])
                .header(Row::new(vec!["NAME", "", "SHARE", "SIZE", "TOUCHED"]).fg(DIM))
                .row_highlight_style(Style::new().bg(LINE))
                .block(Self::panel(&format!("{crumbs} · {}", human_bytes(node.bytes)))),
            main,
            &mut self.tstate,
        );
        let mut lines = vec![
            Line::from(vec!["files ".fg(DIM), format!("{}", node.files).into()]),
            Line::from(vec!["dirs  ".fg(DIM), format!("{}", node.dirs).into()]),
            Line::from(vec!["small ".fg(DIM), human_bytes(node.other_bytes).into(), " in folded entries".fg(DIM)]),
            Line::raw(""),
            Line::from("LARGEST FILES HERE").fg(DIM).bold(),
        ];
        for (n, b) in &node.top_files {
            lines.push(Line::from(vec![format!("{:>9}  ", human_bytes(*b)).fg(ACCENT), n.clone().into()]));
        }
        f.render_widget(Paragraph::new(lines).block(Self::panel("Details")).wrap(Wrap { trim: true }), side);
    }

    fn draw_advisor(&self, f: &mut Frame, a: Rect) {
        let [head, body] = Layout::vertical([Constraint::Length(4), Constraint::Min(5)]).areas(a);
        let pname = match self.provider {
            Provider::ClaudeCode => "Claude Code (local `claude -p`, no key)",
            Provider::Anthropic => "Anthropic API (ANTHROPIC_API_KEY)",
            Provider::OpenAiCompat => "Agnes / OpenAI-compatible (AGNES_API_KEY)",
            Provider::Offline => "Offline rules",
        };
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![" provider ".fg(DIM), pname.fg(VIOLET).bold(), "   p to change".fg(DIM)]),
                Line::from(" The model only sees sizes, ages, safety labels and finding ids — it ranks, it never deletes. Every action still needs your confirmation.").fg(DIM),
            ])
            .block(Self::panel("Advisor")),
            head,
        );
        let mut lines = vec![];
        if let Some(t0) = self.advising {
            let dots = ".".repeat((t0.elapsed().as_millis() / 400) as usize % 4);
            lines.push(Line::from(format!(" thinking about {} findings{dots}  {:.0}s", self.report.as_ref().map(|r| r.findings.len()).unwrap_or(0), t0.elapsed().as_secs_f32())).fg(ACCENT));
        } else {
            match &self.advice {
                None => lines.push(Line::from(" Press a to ask the advisor what to reclaim first and how to stop it coming back.").fg(DIM)),
                Some(Err(e)) => lines.push(Line::from(format!(" ✗ {e}")).fg(ROSE)),
                Some(Ok(adv)) => {
                    lines.push(Line::from(vec![" ".into(), adv.summary.clone().bold()]));
                    lines.push(Line::from(format!(" via {}", adv.provider)).fg(DIM));
                    lines.push(Line::raw(""));
                    let r = self.report.as_ref();
                    for rec in &adv.recommendations {
                        let f = r.and_then(|r| r.findings.iter().find(|f| f.id == rec.id));
                        let vc = match rec.verdict.as_str() {
                            "reclaim" => ACCENT,
                            "relocate" => SKY,
                            "keep" => DIM,
                            _ => AMBER,
                        };
                        lines.push(Line::from(vec![
                            format!(" P{} ", rec.priority).fg(BG).bg(vc),
                            format!(" {:<9}", rec.verdict).fg(vc),
                            format!("{:>9}  ", f.map(|f| human_bytes(f.bytes)).unwrap_or_default()).into(),
                            f.map(|f| f.title.clone()).unwrap_or_default().bold(),
                        ]));
                        lines.push(Line::from(format!("               {}", rec.rationale)).fg(DIM));
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::from(" PREVENT IT COMING BACK").fg(DIM).bold());
                    for p in &adv.prevention {
                        lines.push(Line::from(vec![" › ".fg(ACCENT), p.clone().into()]));
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::from(" s selects the reclaim picks in Findings").fg(DIM));
                }
            }
        }
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(Self::panel("Recommendations")), body);
    }

    fn draw_modal(&self, f: &mut Frame, area: Rect) {
        let (title, lines, w, h): (String, Vec<Line>, u16, u16) = match &self.modal {
            Modal::None => return,
            Modal::Help => (
                "Keys".into(),
                vec![
                    Line::from("1-6 / tab   switch view          r   rescan (low priority)"),
                    Line::from("q           quit                 ?   this help"),
                    Line::raw(""),
                    Line::from("Findings    space toggle · p recommended · x clear · / filter"),
                    Line::from("            enter details · c preview cleanup → y confirm"),
                    Line::from("Resources   c/m/t/i sort · a agents only · K kill"),
                    Line::from("Explorer    enter/→ open · backspace/← up · R rescan"),
                    Line::from("Advisor     a ask · p provider · s select picks"),
                    Line::raw(""),
                    Line::from("Nothing is ever deleted without a dry-run preview and your y.").fg(DIM),
                ],
                70,
                14,
            ),
            Modal::Detail(id) => {
                let Some(fi) = self.report.as_ref().and_then(|r| r.findings.iter().find(|f| &f.id == id)) else { return };
                let mut l = vec![
                    Line::from(fi.title.clone()).bold(),
                    Line::from(fi.path.display().to_string()).fg(SKY),
                    Line::raw(""),
                    Line::from(vec!["size     ".fg(DIM), human_bytes(fi.bytes).into(), format!("  ({} files)", fi.files).fg(DIM)]),
                    Line::from(vec!["safety   ".fg(DIM), format!("{:?}", fi.safety).fg(safety_color(fi.safety))]),
                    Line::from(vec!["owner    ".fg(DIM), fi.owner.clone().unwrap_or("—".into()).fg(VIOLET)]),
                    Line::from(vec!["idle     ".fg(DIM), fi.age_days(self.report.as_ref().unwrap().generated_at).map(idle).unwrap_or("—".into()).into()]),
                    Line::from(vec!["action   ".fg(DIM), action_text(&fi.action).into()]),
                ];
                if let Some(p) = &fi.protected {
                    l.push(Line::from(vec!["guard    ".fg(DIM), format!("🔒 {p}").fg(AMBER)]));
                }
                if let Some(rt) = &fi.relocate_to {
                    l.push(Line::from(vec!["or move  ".fg(DIM), rt.display().to_string().fg(SKY)]));
                }
                l.push(Line::raw(""));
                l.push(Line::from(fi.reason.clone()).fg(DIM));
                ("Finding".into(), l, 90, 16)
            }
            Modal::Confirm { outcomes, .. } | Modal::Done(outcomes) => {
                let done = matches!(self.modal, Modal::Done(_));
                let total: u64 = outcomes.iter().filter(|o| o.ok).map(|o| o.freed).sum();
                let mut l = vec![Line::from(vec![
                    if done { "Freed ".into() } else { "Will free ".into() },
                    human_bytes(total).fg(ACCENT).bold(),
                    format!(" across {} items", outcomes.iter().filter(|o| o.ok).count()).into(),
                ])];
                l.push(Line::raw(""));
                for o in outcomes.iter().take(18) {
                    l.push(Line::from(vec![if o.ok { " ✔ ".fg(ACCENT) } else { " ✗ ".fg(ROSE) }, format!("{:<44}", trunc(&o.title, 44)).into(), trunc(&o.message, 60).fg(DIM)]));
                    if let Some(m) = &o.manual {
                        l.push(Line::from(format!("     run yourself: {m}")).fg(AMBER));
                    }
                }
                l.push(Line::raw(""));
                l.push(if done { Line::from("any key to close").fg(DIM) } else { Line::from(vec![" y ".fg(BG).bg(ACCENT), " do it    ".into(), " esc ".fg(TEXT).bg(LINE), " cancel".into()]) });
                (if done { "Cleanup finished".into() } else { "Preview — nothing deleted yet".into() }, l, 116, 26)
            }
            Modal::Kill(pid, name) => (
                "Stop process".into(),
                vec![
                    Line::from(format!("{name} (pid {pid})")).bold(),
                    Line::raw(""),
                    Line::from(vec![" y ".fg(BG).bg(AMBER), " SIGTERM   ".into(), " K ".fg(BG).bg(ROSE), " SIGKILL   ".into(), " esc ".fg(TEXT).bg(LINE), " cancel".into()]),
                ],
                56,
                7,
            ),
        };
        let w = w.min(area.width - 4);
        let h = h.min(area.height - 2);
        let r = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
        f.render_widget(Clear, r);
        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(ACCENT)).title(format!(" {title} ")).style(Style::new().bg(PANEL).fg(TEXT)).padding(ratatui::widgets::Padding::horizontal(1))),
            r,
        );
    }
}

thread_local! {
    static RX: std::cell::RefCell<Option<Receiver<Msg>>> = const { std::cell::RefCell::new(None) };
}

fn push(q: &mut VecDeque<u64>, v: u64) {
    if q.len() == 240 {
        q.pop_front();
    }
    q.push_back(v);
}

fn ago(t: i64) -> String {
    let s = (stratax_core::resources::now() - t).max(0);
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", s / 60),
        3600..=86_399 => format!("{}h ago", s / 3600),
        _ => format!("{}d ago", s / 86_400),
    }
}

fn idle(d: f64) -> String {
    if d < 1.0 / 24.0 {
        "active".into()
    } else if d < 1.0 {
        format!("{:.0}h", d * 24.0)
    } else {
        format!("{:.0}d", d)
    }
}

fn uptime(s: u64) -> String {
    format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
}

fn short_home(p: &std::path::Path) -> String {
    let h = stratax_core::platform::home();
    p.strip_prefix(&h).map(|r| format!("~/{}", r.display())).unwrap_or_else(|_| p.display().to_string())
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

fn action_text(a: &stratax_core::Action) -> String {
    use stratax_core::Action::*;
    match a {
        Delete { paths } if paths.len() == 1 => "delete".into(),
        Delete { paths } => format!("empty ({} entries)", paths.len()),
        RunTool { program, args } => format!("run `{program} {}`", args.join(" ")),
        WorktreeRemove { repo } => format!("git worktree remove (repo {})", repo.display()),
        Relocate { suggested_root } => format!("move to {}", suggested_root.display()),
        ReviewOnly => "review only — nothing automatic".into(),
    }
}
