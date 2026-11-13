// Tauri shell around stratax-core. Every command is a thin, blocking call run
// off the main thread; long ones stream progress as `scan-progress` events.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use stratax_core::{advisor, chat, clean, resources::Sampler, scan, walk, Action, Report};
use tauri::{Emitter, State};

#[derive(Default)]
struct AppState {
    report: Mutex<Option<Report>>,
    sampler: Mutex<Option<Sampler>>,
    progress: Mutex<Option<Arc<walk::Progress>>>,
    chat_cancel: Mutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
}

type R<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Last saved report — lets the UI paint instantly before a fresh scan.
#[tauri::command]
fn last_report(state: State<'_, AppState>) -> Option<Report> {
    let r = scan::load_last();
    *state.report.lock().unwrap() = r.clone();
    r
}

#[tauri::command]
async fn run_scan(app: tauri::AppHandle, state: State<'_, AppState>, roots: Option<Vec<String>>) -> R<Report> {
    let progress = Arc::new(walk::Progress::default());
    *state.progress.lock().unwrap() = Some(progress.clone());
    let mut opts = scan::ScanOptions { progress: progress.clone(), ..Default::default() };
    if let Some(r) = roots.filter(|r| !r.is_empty()) {
        opts.roots = r.into_iter().map(PathBuf::from).collect();
    }
    let ticker = spawn_ticker(app.clone(), progress.clone());
    let report = tauri::async_runtime::spawn_blocking(move || scan::scan(&opts)).await.map_err(err)?;
    progress.cancel.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    *state.report.lock().unwrap() = Some(report.clone());
    Ok(report)
}

/// Full tree of a directory for the explorer (pass 2).
#[tauri::command]
async fn get_tree(app: tauri::AppHandle, path: String) -> R<walk::DirNode> {
    let progress = Arc::new(walk::Progress::default());
    let ticker = spawn_ticker(app, progress.clone());
    let p = progress.clone();
    let node = tauri::async_runtime::spawn_blocking(move || stratax_core::full_tree(&PathBuf::from(path), p)).await.map_err(err)?;
    progress.cancel.store(true, Ordering::Relaxed);
    let _ = ticker.join();
    Ok(node)
}

/// Used bytes per mount at every past scan (≤ 30 days).
#[tauri::command]
fn get_history() -> Vec<stratax_core::history::Point> {
    stratax_core::history::series()
}

#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) {
    if let Some(p) = state.progress.lock().unwrap().as_ref() {
        p.cancel.store(true, Ordering::Relaxed);
    }
}

fn spawn_ticker(app: tauri::AppHandle, p: Arc<walk::Progress>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // `cancel` doubles as "done" for the ticker; the walker only reads it.
        while !p.cancel.load(Ordering::Relaxed) {
            let _ = app.emit(
                "scan-progress",
                serde_json::json!({"files": p.files.load(Ordering::Relaxed), "bytes": p.bytes.load(Ordering::Relaxed)}),
            );
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    })
}

#[tauri::command]
fn get_resources(state: State<'_, AppState>) -> R<stratax_core::resources::ResourceSnapshot> {
    let mut guard = state.sampler.lock().unwrap();
    let s = guard.get_or_insert_with(Sampler::new);
    Ok(s.sample())
}

#[tauri::command]
fn kill_process(pid: u32, force: bool) -> R<()> {
    stratax_core::resources::signal(pid, force).map_err(err)
}

#[tauri::command]
fn get_advisor_config() -> advisor::AdvisorConfig {
    advisor::AdvisorConfig::load()
}

#[tauri::command]
fn set_advisor_config(config: advisor::AdvisorConfig) -> R<()> {
    let dir = stratax_core::platform::home().join(".config/stratax");
    std::fs::create_dir_all(&dir).map_err(err)?;
    std::fs::write(dir.join("advisor.json"), serde_json::to_vec_pretty(&config).map_err(err)?).map_err(err)
}

#[tauri::command]
async fn advise(state: State<'_, AppState>, config: Option<advisor::AdvisorConfig>) -> R<advisor::Advice> {
    let report = state.report.lock().unwrap().clone().ok_or("run a scan first")?;
    let cfg = config.unwrap_or_else(advisor::AdvisorConfig::load);
    tauri::async_runtime::spawn_blocking(move || advisor::advise(&report, &cfg)).await.map_err(err)?.map_err(err)
}

/// One conversational turn. Text streams as `advisor-chunk` events
/// `{id, text}`; the full reply is also returned.
#[tauri::command]
async fn advisor_chat(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    messages: Vec<chat::ChatMsg>,
    config: Option<advisor::AdvisorConfig>,
) -> R<String> {
    let report = state.report.lock().unwrap().clone().ok_or("run a scan first")?;
    let live = {
        let mut guard = state.sampler.lock().unwrap();
        // CPU usage needs two readings: warm up a sampler that was just created.
        if guard.is_none() {
            let mut s = Sampler::new();
            s.sample();
            std::thread::sleep(std::time::Duration::from_millis(300));
            *guard = Some(s);
        }
        guard.as_mut().unwrap().sample()
    };
    let cfg = config.unwrap_or_else(advisor::AdvisorConfig::load);
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    *state.chat_cancel.lock().unwrap() = Some(cancel.clone());
    tauri::async_runtime::spawn_blocking(move || {
        let live = chat::live_context(&live);
        chat::chat(&report, Some(&live), &messages, &cfg, &cancel, &mut |t| {
            let _ = app.emit("advisor-chunk", serde_json::json!({"id": id, "text": t}));
        })
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
fn cancel_chat(state: State<'_, AppState>) {
    if let Some(c) = state.chat_cancel.lock().unwrap().as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

/// `mode`: "reclaim" (the finding's own action) or "relocate".
#[tauri::command]
async fn clean(state: State<'_, AppState>, ids: Vec<String>, dry_run: bool, mode: Option<String>) -> R<Vec<clean::Outcome>> {
    let report = state.report.lock().unwrap().clone().ok_or("run a scan first")?;
    let relocate = mode.as_deref() == Some("relocate");
    let picked: Vec<_> = report
        .findings
        .into_iter()
        .filter(|f| ids.contains(&f.id))
        .map(|mut f| {
            if relocate {
                if let Some(root) = f.relocate_to.clone() {
                    f.action = Action::Relocate { suggested_root: root };
                }
            }
            f
        })
        .collect();
    tauri::async_runtime::spawn_blocking(move || clean::execute(&picked, dry_run)).await.map_err(err)
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            last_report, get_history, run_scan, get_tree, cancel_scan, get_resources, kill_process,
            get_advisor_config, set_advisor_config, advise, advisor_chat, cancel_chat, clean
        ])
        .run(tauri::generate_context!())
        .expect("error while running Stratax");
}
