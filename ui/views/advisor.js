import { S, go, toast, touchSelection } from '../app.js';
import { api } from '../api.js';
import { t, fmtInt } from '../i18n.js';
import { icon, esc, B, L, verdictBadge, rich, codeLine, safetyBadge } from '../ui.js';
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
const V = { loaded: false, choice: 'claude_code', form: { ...DEFAULTS.claude_code }, result: null, loading: false, error: null, started: 0, timer: null };

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
      <button class="btn primary" data-act="adv-ask" ${V.loading || !S.report ? 'disabled' : ''}>${icon('advisor')}${esc(t('adv.ask'))}</button>
      <button class="btn ghost" data-act="adv-save">${esc(t('adv.save_default'))}</button>
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
  const snippets = [...p.matchAll(/`([^`]+)`/g)].map((m) => m[1]).filter((s) => s.length > 14);
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

function rightHtml() {
  if (V.loading) return thinkHtml();
  if (V.error) return `<div class="err">${icon('critical')}<div><b>${esc(t('common.error'))}</b><div class="muted" style="font-family:var(--mono);font-size:12px;direction:ltr;text-align:start">${esc(V.error)}</div></div></div>`;
  if (V.result) return resultHtml(V.result);
  return `<div class="empty" style="padding-block-start:8px"><h3>${esc(t('adv.empty_t'))}</h3><p>${esc(t('adv.empty_b'))}</p></div>`;
}

async function ask() {
  V.loading = true; V.error = null; V.started = Date.now();
  view.render(document.getElementById('view'));
  clearInterval(V.timer);
  V.timer = setInterval(() => {
    if (S.view !== 'advisor' || !V.loading) return;
    const r = document.getElementById('adv-right');
    if (r) r.innerHTML = thinkHtml();
  }, 1000);
  try {
    V.result = await api.advise(toConfig());
  } catch (e) {
    V.error = String(e?.message || e);
  } finally {
    V.loading = false; clearInterval(V.timer);
    if (S.view === 'advisor') view.render(document.getElementById('view'));
  }
}

const view = {
  title: () => t('adv.title'),
  sub: () => esc(t('adv.sub')),
  noScanBtn: true,
  render(el) {
    if (!V.loaded) { loadConfig(); }
    el.innerHTML = `<div class="adv-grid"><div>${formHtml()}</div><div id="adv-right">${rightHtml()}</div></div>`;
  },
  acts: {
    'adv-choice': (el) => { V.choice = el.value; V.form = { ...DEFAULTS[V.choice] }; view.render(document.getElementById('view')); },
    'adv-field': (el) => { V.form[el.dataset.f] = el.value; },
    'adv-ask': () => ask(),
    'adv-save': async () => {
      try { await api.setAdvisorConfig(toConfig()); toast(t('common.saved')); } catch (e) { toast(String(e?.message || e), true); }
    },
    'adv-select': (el) => {
      const id = el.dataset.id; touchSelection(); S.selected.add(id); revealFindings([id]); go('findings');
    },
    'adv-select-all': () => {
      const byId = new Map(S.report.findings.map((f) => [f.id, f]));
      const ids = V.result.recommendations.filter((r) => r.verdict === 'reclaim' && byId.has(r.id) && !byId.get(r.id).protected).map((r) => r.id);
      touchSelection(); ids.forEach((id) => S.selected.add(id));
      toast(t('adv.selected_n', { count: ids.length }));
      revealFindings(ids); go('findings');
    },
  },
};
export default view;
