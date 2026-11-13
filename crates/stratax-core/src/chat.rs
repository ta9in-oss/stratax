//! Free-form conversation with the advisor. Same providers and the same rule
//! as `advisor`: the model only talks. It gets no tools, refers to findings by
//! id, and every action still goes through the UI's preview and guards.
//!
//! Replies stream through `on_text`; `cancel` stops a reply mid-way.

use crate::advisor::{AdvisorConfig, Provider};
use crate::model::*;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMsg {
    /// "user" or "assistant".
    pub role: String,
    pub content: String,
}

const SYSTEM: &str = "You are the advisor inside Stratax, a disk and resource analyzer for developers whose machines fill up because many AI coding agents (Claude Code, Codex, Cursor) build, test and mutation-test many projects in parallel. You are talking with the owner of this machine.

Below is the latest scan of the machine (disks, space by category, findings, running agents, growth) and, when available, a live snapshot of CPU, memory and processes. Answer from that data. Say plainly when the data does not cover a question rather than guessing.

How to answer:
- Reply in the language the user writes in (Arabic or English included). Use Markdown: short paragraphs, lists, and fenced code blocks for commands.
- When you mention a finding, write its id in square brackets, e.g. [f83e5b749a01a]. Only use ids that appear in the scan. The app turns them into links the user can select and clean.
- You cannot run anything, delete anything or see file contents. Never say you did. To act, the user selects findings in the app (which previews every cleanup first) or runs the commands you suggest.
- Treat `protected` findings as in use: explain why, never suggest removing them. Treat `review` findings as possibly holding real data: say what to check first.
- Prefer explanations that also stop the space coming back (why it regrows, config or habit changes), and moving bulky data to a roomier disk over deleting it when that disk exists.";

/// Live process snapshot in a compact form (top consumers + agent groups).
pub fn live_context(s: &crate::resources::ResourceSnapshot) -> serde_json::Value {
    let top = |key: fn(&crate::resources::ProcInfo) -> f64| {
        let mut v: Vec<_> = s.processes.iter().collect();
        v.sort_by(|a, b| key(b).total_cmp(&key(a)));
        v.into_iter()
            .take(12)
            .map(|p| serde_json::json!({"pid": p.pid, "name": p.name, "agent": p.agent, "cpu_pct": (p.cpu * 10.0).round() / 10.0, "memory": human_bytes(p.memory), "threads": p.threads, "cmd": p.cmd.chars().take(140).collect::<String>()}))
            .collect::<Vec<_>>()
    };
    serde_json::json!({
        "cpu": {"total_pct": s.cpu_total.round(), "cores": s.cpu_cores.len(), "model": s.cpu_brand, "load": s.load},
        "memory": {"used": human_bytes(s.mem_used), "total": human_bytes(s.mem_total), "swap_used": human_bytes(s.swap_used)},
        "processes": s.process_count, "threads": s.thread_count, "zombies": s.zombies,
        "agent_groups": s.agents.iter().map(|a| serde_json::json!({"agent": a.agent, "processes": a.processes, "threads": a.threads, "cpu_pct": a.cpu.round(), "memory": human_bytes(a.memory)})).collect::<Vec<_>>(),
        "top_cpu": top(|p| p.cpu as f64),
        "top_memory": top(|p| p.memory as f64),
    })
}

/// System prompt with the scan appended. Byte-stable for a given report, so
/// provider-side prompt caching hits on every turn of a conversation.
fn system_prompt(r: &Report) -> String {
    let mut ctx = crate::advisor::context(r);
    // Chat questions roam: give the model more of the long tail than `advise`.
    ctx["findings"] = serde_json::Value::Array(
        r.findings
            .iter()
            .take(220)
            .map(|f| serde_json::json!({"id": f.id, "title": f.title, "category": f.category, "owner": f.owner, "safety": f.safety, "size": human_bytes(f.bytes), "age_days": f.age_days(r.generated_at).map(|d| d.round()), "protected": f.protected}))
            .collect(),
    );
    format!("{SYSTEM}\n\n<scan generated_at=\"{}\">\n{}\n</scan>", r.generated_at, ctx)
}

pub fn chat(
    r: &Report,
    live: Option<&serde_json::Value>,
    msgs: &[ChatMsg],
    cfg: &AdvisorConfig,
    cancel: &AtomicBool,
    on_text: &mut dyn FnMut(&str),
) -> anyhow::Result<String> {
    if msgs.last().map(|m| m.role.as_str()) != Some("user") {
        anyhow::bail!("the conversation must end with a user message");
    }
    let system = system_prompt(r);
    // Live numbers change every second; send them with the newest message so
    // the cached prefix (system + earlier turns) stays identical.
    let mut msgs = msgs.to_vec();
    if let Some(l) = live {
        let last = msgs.last_mut().unwrap();
        last.content = format!("<live_snapshot>\n{l}\n</live_snapshot>\n\n{}", last.content);
    }
    let out = match cfg.provider {
        Provider::Offline => {
            let t = "Offline rules can rank findings but cannot hold a conversation. Pick Claude Code, the Anthropic API or an OpenAI-compatible provider (e.g. Agnes) to chat.";
            on_text(t);
            t.to_string()
        }
        Provider::ClaudeCode => claude_code(&system, &msgs, cfg, cancel, on_text)?,
        Provider::Anthropic => anthropic(&system, &msgs, cfg, cancel, on_text)?,
        Provider::OpenAiCompat => openai_compat(&system, &msgs, cfg, cancel, on_text)?,
    };
    Ok(out)
}

fn claude_code(system: &str, msgs: &[ChatMsg], cfg: &AdvisorConfig, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> anyhow::Result<String> {
    // `claude -p` takes one prompt: replay the transcript in it. Resending the
    // whole conversation (instead of --resume) keeps all providers identical.
    let mut prompt = String::new();
    if msgs.len() > 1 {
        prompt.push_str("<conversation_so_far>\n");
        for m in &msgs[..msgs.len() - 1] {
            prompt.push_str(&format!("<{0}>\n{1}\n</{0}>\n", m.role, m.content));
        }
        prompt.push_str("</conversation_so_far>\n\nReply to the user's new message:\n\n");
    }
    prompt.push_str(&msgs.last().unwrap().content);

    let mut cmd = crate::advisor::isolated_claude();
    cmd.args(["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--no-session-persistence", "--tools", "", "--system-prompt", system]);
    if let Some(m) = &cfg.model {
        cmd.args(["--model", m]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("could not start `claude` (is Claude Code installed?): {e}"))?;
    child.stdin.take().unwrap().write_all(prompt.as_bytes())?;
    let mut text = String::new();
    let mut result = None;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            break;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line?) else { continue };
        match v["type"].as_str() {
            Some("stream_event") if v["event"]["type"] == "content_block_delta" && v["event"]["delta"]["type"] == "text_delta" => {
                if let Some(t) = v["event"]["delta"]["text"].as_str() {
                    text.push_str(t);
                    on_text(t);
                }
            }
            Some("result") => {
                if v["is_error"].as_bool() == Some(true) {
                    result = Some(Err(v["result"].as_str().unwrap_or("error").to_string()));
                } else {
                    result = Some(Ok(()));
                }
            }
            _ => {}
        }
    }
    let status = child.wait()?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(text);
    }
    match result {
        Some(Err(e)) => anyhow::bail!("claude -p: {e}"),
        None if !status.success() => {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut err);
            }
            anyhow::bail!("claude -p failed: {}", err.trim())
        }
        _ => Ok(text),
    }
}

fn anthropic(system: &str, msgs: &[ChatMsg], cfg: &AdvisorConfig, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> anyhow::Result<String> {
    let env = cfg.api_key_env.as_deref().unwrap_or("ANTHROPIC_API_KEY");
    let key = std::env::var(env).map_err(|_| anyhow::anyhow!("{env} is not set"))?;
    let base = cfg.base_url.as_deref().unwrap_or("https://api.anthropic.com");
    let body = serde_json::json!({
        "model": cfg.model.as_deref().unwrap_or("claude-opus-5-5"),
        "max_tokens": 64000,
        "stream": true,
        "system": system,
        // Chat: low effort answers quickly; thinking stays adaptive (it can't be disabled on Opus 5.5).
        "output_config": {"effort": "low"},
        "cache_control": {"type": "ephemeral"},
        "fallbacks": "default",
        "messages": msgs.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})).collect::<Vec<_>>(),
    });
    let resp = ureq::post(&format!("{base}/v1/messages"))
        .set("x-api-key", &key)
        .set("anthropic-version", "2026-10-21")
        .set("anthropic-beta", "server-side-fallback-2026-07-01")
        .set("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(600))
        .send_json(body)
        .map_err(crate::advisor::http_err)?;
    let mut text = String::new();
    for line in BufReader::new(resp.into_reader()).lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line?;
        let Some(data) = line.strip_prefix("data:") else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data.trim()) else { continue };
        match v["type"].as_str() {
            Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                if let Some(t) = v["delta"]["text"].as_str() {
                    text.push_str(t);
                    on_text(t);
                }
            }
            Some("message_delta") if v["delta"]["stop_reason"] == "refusal" => {
                anyhow::bail!("the model declined: {}", v["delta"]["stop_details"]["explanation"].as_str().unwrap_or("no details"));
            }
            Some("error") => anyhow::bail!("API error: {}", v["error"]["message"].as_str().unwrap_or("unknown")),
            _ => {}
        }
    }
    Ok(text)
}

fn openai_compat(system: &str, msgs: &[ChatMsg], cfg: &AdvisorConfig, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> anyhow::Result<String> {
    let env = cfg.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY");
    let key = std::env::var(env).map_err(|_| anyhow::anyhow!("{env} is not set"))?;
    let base = cfg.base_url.as_deref().unwrap_or("https://apihub.agnes-ai.com/v1");
    let mut messages = vec![serde_json::json!({"role": "system", "content": system})];
    messages.extend(msgs.iter().map(|m| serde_json::json!({"role": m.role, "content": m.content})));
    let body = serde_json::json!({
        "model": cfg.model.as_deref().unwrap_or("agnes-2.5-pro-alpha"),
        "messages": messages,
        "stream": true,
    });
    let resp = ureq::post(&format!("{}/chat/completions", base.trim_end_matches('/')))
        .set("authorization", &format!("Bearer {key}"))
        .timeout(std::time::Duration::from_secs(600))
        .send_json(body)
        .map_err(crate::advisor::http_err)?;
    let mut text = String::new();
    for line in BufReader::new(resp.into_reader()).lines() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let line = line?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else { continue };
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
        if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
            text.push_str(t);
            on_text(t);
        }
    }
    Ok(text)
}

/// Finding ids the reply mentions, in order, deduplicated, real ones only.
pub fn referenced_ids(r: &Report, text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in text.split('[').skip(1) {
        if let Some(id) = part.split(']').next() {
            if id.starts_with('f') && id.len() == 13 && !out.iter().any(|x| x == id) && r.findings.iter().any(|f| f.id == id) {
                out.push(id.to_string());
            }
        }
    }
    out
}
