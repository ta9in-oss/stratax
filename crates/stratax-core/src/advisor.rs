//! AI advisor. The model sees a compact summary of the scan and answers with
//! rankings **by finding id only** — it can never name a path to delete, and
//! every action still goes through the guards and the user's confirmation.

use crate::model::*;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// Local `claude -p` — uses the user's Claude Code login, no key needed.
    ClaudeCode,
    /// Anthropic Messages API (`ANTHROPIC_API_KEY`).
    Anthropic,
    /// Any OpenAI-compatible chat endpoint, e.g. Agnes (`AGNES_API_KEY`).
    OpenAiCompat,
    /// Deterministic rules, no network.
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisorConfig {
    pub provider: Provider,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key_env: Option<String>,
}

impl Default for AdvisorConfig {
    fn default() -> Self {
        Self {
            provider: Provider::ClaudeCode,
            model: None,
            base_url: None,
            api_key_env: None,
        }
    }
}

impl AdvisorConfig {
    pub fn agnes() -> Self {
        Self {
            provider: Provider::OpenAiCompat,
            model: Some("agnes-2.5-pro-alpha".into()),
            base_url: Some("https://apihub.agnes-ai.com/v1".into()),
            api_key_env: Some("AGNES_API_KEY".into()),
        }
    }

    /// `~/.config/stratax/advisor.json`, then env overrides, then default.
    pub fn load() -> Self {
        let path = crate::platform::home().join(".config/stratax/advisor.json");
        let mut c: AdvisorConfig = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        match std::env::var("STRATAX_PROVIDER").ok().as_deref() {
            Some("agnes") => c = Self::agnes(),
            Some("anthropic") => c.provider = Provider::Anthropic,
            Some("claude-code") => c.provider = Provider::ClaudeCode,
            Some("openai") => c.provider = Provider::OpenAiCompat,
            Some("offline") => c.provider = Provider::Offline,
            _ => {}
        }
        if let Ok(m) = std::env::var("STRATAX_MODEL") {
            c.model = Some(m);
        }
        c
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Advice {
    #[serde(default)]
    pub provider: String,
    pub summary: String,
    pub recommendations: Vec<Recommendation>,
    /// Habits / config changes that stop the space coming back.
    #[serde(default)]
    pub prevention: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub id: String,
    /// 1 = do first.
    pub priority: u8,
    /// reclaim | relocate | keep | review
    pub verdict: String,
    pub rationale: String,
}

const SYSTEM: &str = "You are the disk-space advisor inside Stratax, a tool for developers whose machines fill up because many AI coding agents (Claude Code, Codex, Cursor) build, test and mutation-test many projects in parallel.

You get a JSON scan: disks, category totals, findings (each with an id, safety label safe|rebuild|review, age, owner, whether a guard protects it) and running agent processes.

Decide what the user should reclaim first and how to stop it coming back. Rules:
- Refer to findings ONLY by their `id`. Never invent ids or paths.
- Never recommend reclaiming a finding that has `protected` set.
- `review` findings may hold real data: verdict `review` or `keep` unless clearly disposable, and explain what to check.
- Prefer `relocate` for large, rarely-written data when an idle or unmounted HDD exists.
- Think about why each item regrows (mutation runs rebuilding target/, agents downloading into session temp, docker builds) and put concrete fixes in `prevention` (env vars, config lines, cron-able commands).
- Rank by bytes freed per unit of risk. 5–15 recommendations.
Answer with JSON only: {\"summary\": string, \"recommendations\": [{\"id\": string, \"priority\": 1-5, \"verdict\": \"reclaim\"|\"relocate\"|\"keep\"|\"review\", \"rationale\": string}], \"prevention\": [string]}";

fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "recommendations", "prevention"],
        "properties": {
            "summary": {"type": "string"},
            "recommendations": {"type": "array", "items": {
                "type": "object", "additionalProperties": false,
                "required": ["id", "priority", "verdict", "rationale"],
                "properties": {
                    "id": {"type": "string"},
                    "priority": {"type": "integer"},
                    "verdict": {"type": "string", "enum": ["reclaim", "relocate", "keep", "review"]},
                    "rationale": {"type": "string"}
                }
            }},
            "prevention": {"type": "array", "items": {"type": "string"}}
        }
    })
}

/// What the model sees. Paths are shortened to `~` to keep tokens down.
pub fn context(r: &Report) -> serde_json::Value {
    let home = crate::platform::home().display().to_string();
    let short = |p: &std::path::Path| {
        let s = p.display().to_string();
        s.strip_prefix(&home).map(|x| format!("~{x}")).unwrap_or(s)
    };
    serde_json::json!({
        "host": r.host,
        "disks": r.disks.iter().map(|d| serde_json::json!({
            "device": d.device, "kind": d.kind(), "label": d.label, "mount": d.mount,
            "total": human_bytes(d.total), "used_pct": (d.used_ratio() * 100.0).round(), "free": human_bytes(d.available),
        })).collect::<Vec<_>>(),
        "totals": r.totals.iter().map(|t| serde_json::json!({"category": t.label, "size": human_bytes(t.bytes), "reclaimable": human_bytes(t.reclaimable), "items": t.count})).collect::<Vec<_>>(),
        "insights": r.insights.iter().map(|i| format!("{}: {}", i.title, i.detail)).collect::<Vec<_>>(),
        "agents_running": r.agents.iter().map(|a| serde_json::json!({"agent": a.agent, "processes": a.processes, "memory": human_bytes(a.memory), "cwds": a.cwds.iter().take(6).map(|c| short(c)).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "growth": r.growth,
        "findings": r.findings.iter().take(90).map(|f| serde_json::json!({
            "id": f.id, "title": f.title, "path": short(&f.path), "category": f.category, "owner": f.owner,
            "safety": f.safety, "size": human_bytes(f.bytes), "bytes": f.bytes,
            "age_days": f.age_days(r.generated_at).map(|d| (d * 10.0).round() / 10.0),
            "protected": f.protected, "reason": f.reason,
        })).collect::<Vec<_>>(),
    })
}

pub fn advise(r: &Report, cfg: &AdvisorConfig) -> anyhow::Result<Advice> {
    let ctx = serde_json::to_string(&context(r))?;
    let raw = match cfg.provider {
        Provider::Offline => return Ok(offline(r)),
        Provider::ClaudeCode => claude_code(&ctx, cfg)?,
        Provider::Anthropic => anthropic(&ctx, cfg)?,
        Provider::OpenAiCompat => openai_compat(&ctx, cfg)?,
    };
    let mut advice: Advice = serde_json::from_value(extract_json(&raw)?)
        .map_err(|e| anyhow::anyhow!("advisor answered in an unexpected shape: {e}"))?;
    advice.provider = format!(
        "{:?}{}",
        cfg.provider,
        cfg.model
            .as_ref()
            .map(|m| format!(" · {m}"))
            .unwrap_or_default()
    );
    // Drop anything that doesn't map to a real, unprotected finding.
    advice.recommendations.retain(|rec| {
        r.findings.iter().any(|f| {
            f.id == rec.id
                && (f.protected.is_none() || rec.verdict == "keep" || rec.verdict == "review")
        })
    });
    advice.recommendations.sort_by_key(|x| x.priority);
    Ok(advice)
}

/// `claude` without the user's personal setup: user-level plugins and hooks
/// (output styles, session-start context) would otherwise leak into replies,
/// and a project's `.claude/` would apply if Stratax were started inside one.
/// `--bare` would do this too but also skips the login, so it can't be used.
pub(crate) fn isolated_claude() -> Command {
    let dir = crate::history::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let mut cmd = Command::new("claude");
    cmd.current_dir(dir).args(["--setting-sources", "project"]);
    cmd
}

fn claude_code(ctx: &str, cfg: &AdvisorConfig) -> anyhow::Result<String> {
    let mut cmd = isolated_claude();
    cmd.args([
        "-p",
        "--output-format",
        "json",
        "--no-session-persistence",
        "--tools",
        "",
        "--system-prompt",
        SYSTEM,
    ]);
    if let Some(m) = &cfg.model {
        cmd.args(["--model", m]);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            anyhow::anyhow!("could not start `claude` (is Claude Code installed?): {e}")
        })?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("Scan:\n{ctx}").as_bytes())?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!(
            "claude -p failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    if v["is_error"].as_bool() == Some(true) {
        anyhow::bail!("claude -p: {}", v["result"].as_str().unwrap_or("error"));
    }
    Ok(v["result"].as_str().unwrap_or_default().to_string())
}

fn anthropic(ctx: &str, cfg: &AdvisorConfig) -> anyhow::Result<String> {
    let env = cfg.api_key_env.as_deref().unwrap_or("ANTHROPIC_API_KEY");
    let key = std::env::var(env).map_err(|_| anyhow::anyhow!("{env} is not set"))?;
    let base = cfg
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");
    let body = serde_json::json!({
        "model": cfg.model.as_deref().unwrap_or("claude-opus-5-5"),
        "max_tokens": 16000,
        "system": SYSTEM,
        "output_config": {"effort": "medium", "format": {"type": "json_schema", "schema": schema()}},
        "fallbacks": "default",
        "messages": [{"role": "user", "content": format!("Scan:\n{ctx}")}],
    });
    let resp: serde_json::Value = ureq::post(&format!("{base}/v1/messages"))
        .set("x-api-key", &key)
        .set("anthropic-version", "2026-10-21")
        .set("anthropic-beta", "server-side-fallback-2026-07-01")
        .set("content-type", "application/json")
        .timeout(std::time::Duration::from_secs(300))
        .send_json(body)
        .map_err(http_err)?
        .into_json()?;
    if resp["stop_reason"] == "refusal" {
        anyhow::bail!(
            "the model declined: {}",
            resp["stop_details"]["explanation"]
                .as_str()
                .unwrap_or("no details")
        );
    }
    let text: String = resp["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect();
    Ok(text)
}

fn openai_compat(ctx: &str, cfg: &AdvisorConfig) -> anyhow::Result<String> {
    let env = cfg.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY");
    let key = std::env::var(env).map_err(|_| anyhow::anyhow!("{env} is not set"))?;
    let base = cfg
        .base_url
        .as_deref()
        .unwrap_or("https://apihub.agnes-ai.com/v1");
    let body = serde_json::json!({
        "model": cfg.model.as_deref().unwrap_or("agnes-2.5-pro-alpha"),
        "messages": [{"role": "system", "content": SYSTEM}, {"role": "user", "content": format!("Scan:\n{ctx}")}],
        "temperature": 0.2,
    });
    let resp: serde_json::Value =
        ureq::post(&format!("{}/chat/completions", base.trim_end_matches('/')))
            .set("authorization", &format!("Bearer {key}"))
            .timeout(std::time::Duration::from_secs(300))
            .send_json(body)
            .map_err(http_err)?
            .into_json()?;
    Ok(resp["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}

pub(crate) fn http_err(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, r) => anyhow::anyhow!(
            "HTTP {code}: {}",
            r.into_string()
                .unwrap_or_default()
                .chars()
                .take(400)
                .collect::<String>()
        ),
        other => anyhow::anyhow!("{other}"),
    }
}

/// Models sometimes wrap JSON in prose or fences; take the outermost object.
fn extract_json(s: &str) -> anyhow::Result<serde_json::Value> {
    if let Ok(v) = serde_json::from_str(s.trim()) {
        return Ok(v);
    }
    let (a, b) = (s.find('{'), s.rfind('}'));
    match (a, b) {
        (Some(a), Some(b)) if b > a => Ok(serde_json::from_str(&s[a..=b])?),
        _ => anyhow::bail!("advisor did not return JSON"),
    }
}

/// No-network advice: preselected items by size, plus standard prevention.
pub fn offline(r: &Report) -> Advice {
    let mut recs: Vec<Recommendation> = r
        .findings
        .iter()
        .filter(|f| f.protected.is_none() && f.safety != Safety::Review)
        .take(12)
        .map(|f| Recommendation {
            id: f.id.clone(),
            priority: if f.preselected {
                1
            } else if f.safety == Safety::Safe {
                2
            } else {
                3
            },
            verdict: "reclaim".into(),
            rationale: format!("{} · {:?} · {}", human_bytes(f.bytes), f.safety, f.reason),
        })
        .collect();
    recs.sort_by_key(|x| x.priority);
    let mut prevention = vec![
        "Wipe stale build output weekly: `stratax clean --preselected` from cron or a systemd timer.".to_string(),
        "Run mutation tests with a throwaway target dir: `CARGO_TARGET_DIR=/tmp/mutants-target CARGO_INCREMENTAL=0 cargo mutants`, then delete it.".into(),
        "Add a Claude Code SessionEnd hook that removes the session scratchpad under /tmp/claude-<uid> when the session ends.".into(),
        "Cap Docker's build cache: `builder.gc` with `defaultKeepStorage: \"20GB\"` in /etc/docker/daemon.json.".into(),
    ];
    if r.disks.iter().any(|d| d.mount.is_none() && d.rotational) {
        prevention.push("Mount the idle HDD (add it to /etc/fstab) and move Docker's data-root, model caches and old projects onto it.".into());
    }
    Advice {
        provider: "offline rules".into(),
        summary: r
            .insights
            .first()
            .map(|i| i.title.clone())
            .unwrap_or_else(|| "Nothing urgent.".into()),
        recommendations: recs,
        prevention,
    }
}
