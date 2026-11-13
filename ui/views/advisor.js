import { S, go, toast, touchSelection } from '../app.js';
import { api } from '../api.js';
import { t, fmtInt } from '../i18n.js';
import { icon, esc, B, L, verdictBadge, rich, codeLine, safetyBadge, md } from '../ui.js';
import { revealFindings } from './findings.js';

const AGNES = { provider: 'open_ai_compat', model: 'agnes-2.5-pro-alpha', base_url: 'https://apihub.agnes-ai.com/v1', api_key_env: 'AGNES_API_KEY' };
const CHOICES = ['claude_code', 'anthropic', 'agnes', 'open_ai_compat', 'offline'];
const DEFAULTS = {
  claude_code: { model: '', base_url: '', api_key_env: '' },
  anthropic: { model: '', base_url: '', api_key_env: 'ANTHROPIC_API_KEY' },
  agnes: { model: AGNES.model, base_url: AGNES.base_url, api_key_env: AGNES.api_key_env },
  open_ai_compat: { model: '', base_url: 'http://localhost:11434/v1', api_key_env: 'OPENAI_API_KEY' },
  offline: { model: '', base_url: '', api_key_env: '' },
};
const V = { loaded: false, choice: 'claude_code', form: { ...DEFAULTS.claude_code }, loading: false, started: 0, timer: null,
  // Conversation: {role: 'user'|'assistant'|'plan', content, plan?, error?, streaming?}
  thread: restore(), draft: '', streaming: false, showSettings: false };

function restore() { try { return JSON.parse(sessionStorage.getItem('stratax.chat') || '[]').filter((m) => !m.streaming); } catch (_) { return []; } }
function persist() { try { sessionStorage.setItem('stratax.chat', JSON.stringify(V.thread.slice(-60))); } catch (_) { /* storage unavailable */ } }

function choiceOf(cfg) {
  if (!cfg) return 'claude_code';
  if (cfg.provider === 'open_ai_compat' && (cfg.base_url || '').includes('agnes')) return 'agnes';
  return CHOICES.includes(cfg.provider) ? cfg.provider : 'claude_code';
}
function toConfig() {
  const c = V.choice, f = V.form;
  const nz = (s) => (s && s.trim() ? s.trim() : null);
  if (c === 'agnes') return { provider: 'open_ai_compat', model: nz(f.model) || AGNES.model, base_url: nz(f.base_url) || AGNES.base_url, api_key_env: nz(f.api_key_env) || AGNES.api_key_env };
  if (c === 'offline') return { provider: 'offline', model: null, base_url: null, api_key_env: null };
  if (c === 'claude_code') return { provider: 'claude_code', model: nz(f.model), base_url: null, api_key_env: null };
  if (c === 'anthropic') return { provider: 'anthropic', model: nz(f.model), base_url: null, api_key_env: nz(f.api_key_env) };
  return { provider: 'open_ai_compat', model: nz(f.model), base_url: nz(f.base_url), api_key_env: nz(f.api_key_env) };
}

async function loadConfig() {
  try {
    const cfg = await api.getAdvisorConfig();
    V.choice = choiceOf(cfg);
    V.form = { model: cfg.model || DEFAULTS[V.choice].model, base_url: cfg.base_url || DEFAULTS[V.choice].base_url, api_key_env: cfg.api_key_env || DEFAULTS[V.choice].api_key_env };
  } catch (_) { /* keep defaults */ }
  V.loaded = true;
  if (S.view === 'advisor') view.render(document.getElementById('view'));
}

function formHtml() {
  const c = V.choice;
  const showModel = c !== 'offline';
  const showUrl = c === 'open_ai_compat' || c === 'agnes';
  const showKey = c === 'anthropic' || showUrl;
  return `<div class="panel adv-form">
    <div class="providers" role="radiogroup" aria-label="${esc(t('adv.provider'))}">${CHOICES.map((k) => `<label class="prov"><input type="radio" name="prov" value="${k}" data-change="adv-choice" ${V.choice === k ? 'checked' : ''}><b>${esc(t('adv.p_' + k))}</b><span>${esc(t('adv.p_' + k + '_d'))}</span></label>`).join('')}</div>
    ${showModel ? `<label class="field">${esc(t('adv.model'))}<input class="input mono" data-input="adv-field" data-f="model" value="${esc(V.form.model)}" placeholder="${esc(t('adv.model_ph'))}" spellcheck="false"></label>` : ''}
    ${showUrl ? `<label class="field">${esc(t('adv.base_url'))}<input class="input mono" data-input="adv-field" data-f="base_url" value="${esc(V.form.base_url)}" spellcheck="false"></label>` : ''}
    ${showKey ? `<label class="field">${esc(t('adv.key_env'))}<input class="input mono" data-input="adv-field" data-f="api_key_env" value="${esc(V.form.api_key_env)}" spellcheck="false"><span class="hint">${esc(t('adv.key_hint'))}</span></label>` : ''}
    <div class="adv-actions">
      <button class="btn ghost sm" data-act="adv-save">${esc(t('adv.save_default'))}</button>
    </div>
  </div>`;
}

function thinkHtml() {
  const s = Math.floor((Date.now() - V.started) / 1000);
  const marks = [0, 3, 9, 18];
  const n = S.report ? S.report.findings.length : 0;
  const steps = [t('adv.step1', { count: n }), t('adv.step2'), t('adv.step3'), t('adv.step4')];
  const active = marks.filter((m) => s >= m).length - 1;
  return `<div class="panel think" aria-live="polite"><div class="kicker">${esc(t('adv.asking'))} · ${L(toConfig().model || t('adv.p_' + V.choice))}</div>
    <ol>${steps.map((x, i) => `<li class="${i < active ? 'done' : i === active ? 'on' : ''}">${i < active ? icon('check') : i === active ? '<span class="pulse"></span>' : '<span class="ring-dot"></span>'}${esc(x)}</li>`).join('')}</ol>
    <div class="faint" style="font-size:12px" id="adv-elapsed">${esc(t('adv.elapsed', { s: fmtInt(s) }))}</div>
    <div style="display:grid;gap:8px">${'<div class="skel" style="block-size:54px"></div>'.repeat(3)}</div></div>`;
}

function preventItem(p) {
  // Only things that read like a command line get a copyable block (not config keys such as `defaultKeepStorage: "20GB"`).
  const snippets = [...p.matchAll(/`([^`]+)`/g)].map((m) => m[1]).filter((s) => s.length > 14 && /^(sudo\s|[A-Z_][A-Z0-9_]*=|[a-z][\w.+-]*\s)/.test(s));
  return `<li><div>${rich(p)}</div>${snippets.map((s) => codeLine(s)).join('')}</li>`;
}

function resultHtml(a) {
  const byId = new Map((S.report?.findings || []).map((f) => [f.id, f]));
  const bands = [1, 2, 3].map((p) => ({ p, recs: a.recommendations.filter((r) => (r.priority <= 1 ? 1 : r.priority >= 3 ? 3 : 2) === p) })).filter((b) => b.recs.length);
  let rank = 0;
  const reclaimable = a.recommendations.filter((r) => r.verdict === 'reclaim' && byId.has(r.id) && !byId.get(r.id).protected);
  const reclaimBytes = reclaimable.reduce((s, r) => s + byId.get(r.id).bytes, 0);
  return `<div class="panel adv-sum fade-in"><div class="kicker">${esc(t('adv.summary'))} · ${esc(t('adv.by', { provider: '' }).trim())} ${CHOICES.includes(a.provider) ? esc(t('adv.p_' + a.provider)) : L(a.provider)}</div><div class="s-text"><bdi>${rich(a.summary)}</bdi></div></div>
    <div class="section-t">${esc(t('adv.recs'))}<span class="st-sub">${esc(t('common.items', { count: a.recommendations.length }))}</span>
      ${reclaimable.length ? `<button class="btn sm" style="margin-inline-start:auto" data-act="adv-select-all">${icon('check')}${esc(t('adv.select_all'))} ${B(reclaimBytes)}</button>` : ''}</div>
    <div class="recs">${bands.map((b) => `<div class="rec-band">${esc(t('adv.p' + b.p))}</div>${b.recs.map((r) => {
      const f = byId.get(r.id); rank++;
      return `<div class="panel rec fade-in" style="animation-delay:${Math.min(rank, 12) * 0.03}s"><span class="rank">${L(String(rank).padStart(2, '0'))}</span>
        <div class="r-t">${verdictBadge(r.verdict)}<b><bdi>${esc(f ? f.title : t('adv.missing'))}</bdi></b>${f ? safetyBadge(f.safety) : ''}</div>
        <div class="r-end">${f ? B(f.bytes) : ''}${f ? `<button class="btn sm" data-act="adv-select" data-id="${r.id}">${esc(t('adv.select'))}${icon('arrow')}</button>` : ''}</div>
        <div class="r-why"><bdi>${rich(r.rationale)}</bdi></div></div>`;
    }).join('')}`).join('')}</div>
    ${a.prevention.length ? `<div class="section-t">${esc(t('adv.prevent'))}</div><ol class="prev">${a.prevention.map(preventItem).join('')}</ol>` : ''}`;
}

function refChip(id) {
  const f = (S.report?.findings || []).find((x) => x.id === id);
  if (!f) return `<code class="inl">${esc(id)}</code>`;
  return `<button class="fref ${f.protected ? 'locked' : ''}" data-act="adv-ref" data-id="${id}" title="${esc(f.path)}">${f.protected ? icon('lock') : `<i class="sdot" style="background:var(--${f.safety})"></i>`}<bdi>${esc(f.title)}</bdi>${B(f.bytes)}</button>`;
}

function mentioned(text) {
  const byId = new Map((S.report?.findings || []).map((f) => [f.id, f]));
  return [...new Set([...String(text).matchAll(/\[(f[0-9a-f]{12})\]/g)].map((m) => m[1]))].map((id) => byId.get(id)).filter(Boolean);
}

function msgHtml(m, i) {
  if (m.role === 'user') return `<div class="msg user"><div class="bubble" dir="auto">${esc(m.content).replace(/\n/g, '<br>')}</div></div>`;
  if (m.role === 'plan') {
    const body = m.loading ? thinkHtml() : m.error ? errHtml(m.error) : resultHtml(m.plan);
    return `<div class="msg bot plan">${body}</div>`;
  }
  if (m.error) return `<div class="msg bot">${errHtml(m.error)}</div>`;
  const refs = m.streaming ? [] : mentioned(m.content).filter((f) => !f.protected && f.safety !== 'review');
  const caret = m.streaming ? '<span class="caret"></span>' : '';
  return `<div class="msg bot" data-i="${i}"><div class="md">${m.content ? md(m.content, refChip) : ''}${caret}</div>
    ${refs.length > 1 ? `<div class="msg-acts"><button class="btn sm" data-act="adv-select-refs" data-i="${i}">${icon('check')}${esc(t('adv.select_refs', { count: refs.length }))} ${B(refs.reduce((s, f) => s + f.bytes, 0))}</button></div>` : ''}</div>`;
}

function errHtml(e) {
  return `<div class="err">${icon('critical')}<div><b>${esc(t('common.error'))}</b><div class="muted" style="font-family:var(--mono);font-size:12px;direction:ltr;text-align:start">${esc(e)}</div></div></div>`;
}

const SUGGEST = ['adv.q1', 'adv.q2', 'adv.q3', 'adv.q4'];

function threadHtml() {
  if (!V.thread.length) {
    return `<div class="chat-empty"><div class="ce-mark">${icon('advisor')}</div><h3>${esc(t('adv.chat_t'))}</h3><p>${esc(t('adv.chat_b'))}</p>
      <div class="suggest">${SUGGEST.map((k) => `<button class="chip" data-act="adv-suggest" data-k="${k}">${esc(t(k))}</button>`).join('')}
      <button class="chip accent" data-act="adv-plan">${icon('list')}${esc(t('adv.plan'))}</button></div></div>`;
  }
  return V.thread.map(msgHtml).join('');
}

function chatHtml() {
  const prov = t('adv.p_' + V.choice);
  return `<section class="panel chat" aria-label="${esc(t('adv.title'))}">
    <header class="chat-h">
      <button class="chip" data-act="adv-settings" aria-expanded="${V.showSettings}">${icon('settings')}${esc(prov)}${V.form.model ? ` · ${L(V.form.model)}` : ''}</button>
      <span class="spacer"></span>
      <button class="btn sm ghost" data-act="adv-plan" ${V.streaming || !S.report ? 'disabled' : ''}>${icon('list')}${esc(t('adv.plan'))}</button>
      <button class="btn sm ghost" data-act="adv-new" ${V.thread.length && !V.streaming ? '' : 'disabled'}>${icon('plus')}${esc(t('adv.new'))}</button>
    </header>
    <div class="thread" id="adv-thread" aria-live="polite">${threadHtml()}</div>
    <form class="composer" data-submit="adv-send">
      <textarea id="adv-input" rows="1" placeholder="${esc(t(S.report ? 'adv.placeholder' : 'adv.need_scan'))}" data-input="adv-draft" ${S.report ? '' : 'disabled'}>${esc(V.draft)}</textarea>
      ${V.streaming
        ? `<button type="button" class="btn" data-act="adv-stop">${icon('stop')}${esc(t('adv.stop'))}</button>`
        : `<button type="submit" class="btn primary" ${S.report ? '' : 'disabled'} aria-label="${esc(t('adv.send'))}">${icon('send')}<span>${esc(t('adv.send'))}</span></button>`}
    </form>
    <div class="chat-foot faint">${esc(t('adv.foot'))}</div>
  </section>`;
}

function rerenderThread(stick = true) {
  const th = document.getElementById('adv-thread');
  if (!th) return;
  const near = th.scrollHeight - th.scrollTop - th.clientHeight < 80;
  th.innerHTML = threadHtml();
  if (stick || near) th.scrollTop = th.scrollHeight;
}

function full() { if (S.view === 'advisor') { view.render(document.getElementById('view')); const th = document.getElementById('adv-thread'); if (th) th.scrollTop = th.scrollHeight; } }

/** What the model sees: user/assistant turns, with a ranked plan folded into text. */
function transcript() {
  const out = [];
  for (const m of V.thread) {
    if (m.role === 'user') out.push({ role: 'user', content: m.content });
    else if (m.role === 'assistant' && !m.error && m.content) out.push({ role: 'assistant', content: m.content });
    else if (m.role === 'plan' && m.plan) {
      out.push({ role: 'user', content: 'Make a ranked cleanup plan.' });
      out.push({ role: 'assistant', content: `${m.plan.summary}\n\n${m.plan.recommendations.map((r) => `- [${r.id}] ${r.verdict} (P${r.priority}): ${r.rationale}`).join('\n')}\n\nPrevention:\n${m.plan.prevention.map((p) => `- ${p}`).join('\n')}` });
    }
  }
  // Providers need strictly alternating turns that end with the user.
  return out.filter((m, i, a) => !(i > 0 && a[i - 1].role === m.role && m.role === 'assistant'));
}

async function send(text) {
  text = text.trim();
  if (!text || V.streaming || !S.report) return;
  V.thread.push({ role: 'user', content: text });
  const reply = { role: 'assistant', content: '', streaming: true };
  V.thread.push(reply); V.draft = ''; V.streaming = true;
  full();
  let pending = false;
  const paint = () => {
    if (pending) return; pending = true;
    requestAnimationFrame(() => { pending = false; if (!reply.streaming) return; /* final render already done */ const el = document.querySelector(`.msg.bot[data-i="${V.thread.indexOf(reply)}"] .md`); if (el) { el.innerHTML = md(reply.content, refChip) + '<span class="caret"></span>'; const th = document.getElementById('adv-thread'); if (th && th.scrollHeight - th.scrollTop - th.clientHeight < 140) th.scrollTop = th.scrollHeight; } });
  };
  try {
    const finalText = await api.chat(transcript(), toConfig(), (chunk) => { reply.content += chunk; paint(); });
    if (finalText && finalText.length >= reply.content.length) reply.content = finalText;
    if (!reply.content) reply.error = t('adv.no_reply');
  } catch (e) {
    reply.error = String(e?.message || e);
  } finally {
    reply.streaming = false; V.streaming = false; persist(); full();
    document.getElementById('adv-input')?.focus();
  }
}

async function plan() {
  if (V.streaming || !S.report) return;
  const m = { role: 'plan', loading: true };
  V.thread.push(m); V.streaming = true; V.loading = true; V.started = Date.now();
  full();
  clearInterval(V.timer);
  V.timer = setInterval(() => { if (S.view === 'advisor' && m.loading) rerenderThread(false); }, 1000);
  try { m.plan = await api.advise(toConfig()); } catch (e) { m.error = String(e?.message || e); }
  finally { m.loading = false; V.loading = false; V.streaming = false; clearInterval(V.timer); persist(); full(); }
}

function autosize(el) { el.style.blockSize = 'auto'; el.style.blockSize = Math.min(el.scrollHeight, 200) + 'px'; }

const view = {
  title: () => t('adv.title'),
  sub: () => esc(t('adv.sub')),
  noScanBtn: true,
  render(el) {
    if (!V.loaded) { loadConfig(); }
    el.innerHTML = `<div class="adv-grid ${V.showSettings ? 'with-settings' : ''}">${V.showSettings ? `<div class="adv-side">${formHtml()}</div>` : ''}${chatHtml()}</div>`;
    const ta = document.getElementById('adv-input');
    if (ta) {
      autosize(ta);
      ta.onkeydown = (e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(ta.value); } };
      ta.oninput = () => { V.draft = ta.value; autosize(ta); };
    }
    const form = el.querySelector('.composer');
    if (form) form.onsubmit = (e) => { e.preventDefault(); send(ta.value); };
  },
  acts: {
    'adv-choice': (el) => { V.choice = el.value; V.form = { ...DEFAULTS[V.choice] }; full(); },
    'adv-field': (el) => { V.form[el.dataset.f] = el.value; },
    'adv-draft': (el) => { V.draft = el.value; },
    'adv-settings': () => { V.showSettings = !V.showSettings; full(); },
    'adv-save': async () => {
      try { await api.setAdvisorConfig(toConfig()); toast(t('common.saved')); } catch (e) { toast(String(e?.message || e), true); }
    },
    'adv-suggest': (el) => send(t(el.dataset.k)),
    'adv-plan': () => plan(),
    'adv-ask': () => plan(),
    'adv-stop': () => api.cancelChat(),
    'adv-new': () => { V.thread = []; persist(); full(); document.getElementById('adv-input')?.focus(); },
    'adv-ref': (el) => { revealFindings([el.dataset.id]); go('findings'); },
    'adv-select-refs': (el) => {
      const ids = mentioned(V.thread[+el.dataset.i].content).filter((f) => !f.protected && f.safety !== 'review').map((f) => f.id);
      touchSelection(); ids.forEach((id) => S.selected.add(id));
      toast(t('adv.selected_n', { count: ids.length }));
      revealFindings(ids); go('findings');
    },
    'adv-select': (el) => {
      const id = el.dataset.id; touchSelection(); S.selected.add(id); revealFindings([id]); go('findings');
    },
    'adv-select-all': (el) => {
      const byId = new Map(S.report.findings.map((f) => [f.id, f]));
      const planMsg = [...V.thread].reverse().find((m) => m.role === 'plan' && m.plan);
      if (!planMsg) return;
      const ids = planMsg.plan.recommendations.filter((r) => r.verdict === 'reclaim' && byId.has(r.id) && !byId.get(r.id).protected).map((r) => r.id);
      touchSelection(); ids.forEach((id) => S.selected.add(id));
      toast(t('adv.selected_n', { count: ids.length }));
      revealFindings(ids); go('findings');
    },
  },
};
export default view;
