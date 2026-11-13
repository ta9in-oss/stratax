// Stratax app shell: state, routing, scan orchestration, sheets and toasts.
import { api, isMock, onProgress } from './api.js';
import { t, initLang, setLang, getLang, onLang, fmtCompact, fmtBytes, fmtAge, fmtDur } from './i18n.js';
import { icon, esc, B, setHome } from './ui.js';
import overview from './views/overview.js';
import findings from './views/findings.js';
import agents from './views/agents.js';
import resources from './views/resources.js';
import explorer from './views/explorer.js';
import advisor from './views/advisor.js';
import settings from './views/settings.js';
import { cleanupActs } from './views/cleanup.js';

const VIEWS = { overview, findings, agents, resources, explorer, advisor, settings };
const NAV = ['overview', 'findings', 'agents', 'resources', 'explorer', 'advisor'];

export const S = {
  report: null,
  scan: { running: false, files: 0, bytes: 0, error: null, startedAt: 0 },
  selected: new Set(),
  view: 'overview',
  roots: [],
  collapsed: false,
};

const $ = (id) => document.getElementById(id);
const els = { app: $('app'), rail: $('rail'), top: $('topbar'), view: $('view'), layer: $('layer'), toasts: $('toasts') };
let current = null;

// ------------------------------------------------------------ persistence
function store(k, v) { try { localStorage.setItem('stratax.' + k, typeof v === 'string' ? v : JSON.stringify(v)); } catch (_) { /* ignore */ } }
function read(k, d) { try { const v = localStorage.getItem('stratax.' + k); return v == null ? d : v; } catch (_) { return d; } }

export function setTheme(th) { document.documentElement.dataset.theme = th; store('theme', th); renderAll(); }
export function getTheme() { return document.documentElement.dataset.theme || 'dark'; }
export function setRoots(r) { S.roots = r; store('roots', r); }

// ------------------------------------------------------------ report + selection
function adoptReport(r) {
  S.report = r;
  if (r) {
    const homeGuess = r.findings.map((f) => /^\/home\/[^/]+/.exec(f.path)?.[0]).find(Boolean);
    setHome(homeGuess || null);
    const ids = new Set(r.findings.map((f) => f.id));
    if (!S.selectionTouched) {
      S.selected = new Set(r.findings.filter((f) => f.preselected && !f.protected).map((f) => f.id));
    } else {
      S.selected = new Set([...S.selected].filter((id) => ids.has(id)));
    }
  }
}
export function touchSelection() { S.selectionTouched = true; }
export function selectedFindings() { return S.report ? S.report.findings.filter((f) => S.selected.has(f.id)) : []; }

export async function startScan() {
  if (S.scan.running) return;
  S.scan = { running: true, files: 0, bytes: 0, error: null, startedAt: Date.now() };
  S.scanScope = 'scan';
  renderChrome();
  try {
    const r = await api.runScan(S.roots);
    adoptReport(r);
    if (r.scan_status === 'cancelled') toast(t('scan.cancelled'));
  } catch (e) {
    S.scan.error = String(e?.message || e);
    toast(t('scan.failed') + ': ' + S.scan.error, true);
  } finally {
    S.scan.running = false; S.scanScope = null;
    renderAll();
  }
}
export async function cancelScan() { try { await api.cancelScan(); } catch (_) { /* ignore */ } }

onProgress((p) => {
  if (S.scanScope !== 'scan') return; // explorer walks handle their own progress
  S.scan.files = p.files; S.scan.bytes = p.bytes;
  const n = document.getElementById('sc-nums');
  if (n) n.innerHTML = progressText();
});
function progressText() { return `<bdi dir="ltr">${esc(fmtCompact(S.scan.files))}</bdi> · ${B(S.scan.bytes)}`; }

// ------------------------------------------------------------ routing
export function go(view, opts = {}) {
  if (!VIEWS[view]) view = 'overview';
  if (location.hash !== '#' + view) history.replaceState(null, '', '#' + view);
  if (current && current.unmount) current.unmount();
  if (view !== S.view) closeSheet(); // a sheet belongs to the view that opened it
  S.view = view; current = VIEWS[view];
  if (opts.then) S.pending = opts.then;
  renderAll(true);
  els.view.scrollTop = 0;
}
window.addEventListener('hashchange', () => { const v = location.hash.slice(1); if (v !== S.view) go(v); });

// ------------------------------------------------------------ rendering
export function renderAll(fresh = false) {
  renderChrome();
  renderView(fresh);
}
export function renderView(fresh = false) {
  if (!current) return;
  const keep = fresh ? 0 : els.view.scrollTop;
  els.view.className = 'view';
  current.render(els.view, { fresh });
  if (fresh && current.mount) current.mount(els.view);
  if (!fresh) els.view.scrollTop = keep;
  if (S.pending) { const fn = S.pending; S.pending = null; fn(); }
}

export function renderChrome() {
  renderRail();
  renderTop();
}

function counts() {
  const r = S.report;
  return { findings: r ? r.findings.length : null, agents: r ? r.agents.filter((a) => a.kind === 'agent').length || null : null };
}

function renderRail() {
  const c = counts();
  const sc = S.scan;
  const r = S.report;
  let card = '';
  if (sc.running) {
    card = `<div class="scan-card" aria-live="polite"><div class="sc-head"><span>${esc(r ? t('scan.refreshing') : t('scan.scanning'))}</span><button class="btn sm ghost" data-act="cancel-scan">${esc(t('common.stop'))}</button></div>
      <div class="sc-nums" id="sc-nums">${progressText()}</div><div class="scan-track"><i></i></div></div>`;
  } else if (r) {
    card = `<div class="scan-card"><div class="sc-head"><span class="rf-text">${esc(t('scan.last', { age: fmtAge(r.generated_at) }))}</span></div>
      <div class="faint">${esc(t('scan.took', { dur: fmtDur(r.duration_ms / 1000) }))}${r.scan_status !== 'complete' ? ' · ' + esc(t('scan.partial')) : ''}</div></div>`;
  }
  els.rail.innerHTML = `
    <div class="brand"><div class="brand-mark">${icon('mark')}</div><div class="brand-text"><div class="brand-name">${esc(t('app.name'))}</div><div class="brand-sub">${esc(t('app.tagline'))}</div></div></div>
    <nav class="nav" aria-label="Main">
      ${NAV.map((v) => `<a href="#${v}" ${S.view === v ? 'aria-current="page"' : ''} title="${esc(t('nav.' + v))}">${icon(v)}<span class="nav-label">${esc(t('nav.' + v))}</span>${c[v] ? `<span class="nav-count">${esc(fmtCompact(c[v]))}</span>` : ''}</a>`).join('')}
      <div class="nav-sep"></div>
      <a href="#settings" ${S.view === 'settings' ? 'aria-current="page"' : ''} title="${esc(t('nav.settings'))}">${icon('settings')}<span class="nav-label">${esc(t('nav.settings'))}</span></a>
    </nav>
    <div class="rail-foot">
      ${card}
      <div class="rail-tools">
        <button class="btn icon ghost" data-act="toggle-lang" title="${getLang() === 'ar' ? 'English' : 'العربية'}" aria-label="${getLang() === 'ar' ? 'English' : 'العربية'}">${icon('lang')}</button>
        <button class="btn icon ghost" data-act="toggle-theme" title="${esc(t(getTheme() === 'dark' ? 'st.light' : 'st.dark'))}" aria-label="${esc(t(getTheme() === 'dark' ? 'st.light' : 'st.dark'))}">${icon(getTheme() === 'dark' ? 'sun' : 'moon')}</button>
        <button class="btn icon ghost" data-act="toggle-rail" title="${esc(t(S.collapsed ? 'nav.expand' : 'nav.collapse'))}" aria-label="${esc(t(S.collapsed ? 'nav.expand' : 'nav.collapse'))}">${icon('sidebar')}</button>
      </div>
    </div>`;
}

function renderTop() {
  const v = current;
  if (!v) return;
  const sc = S.scan;
  const scanBtn = sc.running
    ? `<button class="btn" data-act="cancel-scan">${icon('stop')}${esc(t('common.stop'))}</button>`
    : `<button class="btn" data-act="rescan">${icon('refresh')}${esc(S.report ? t('common.rescan') : t('common.scan'))}</button>`;
  els.top.innerHTML = `<div><h1>${esc(v.title())}</h1>${v.sub ? `<div class="tb-sub">${v.sub()}</div>` : ''}</div>
    <div class="tb-actions">${isMock ? `<span class="mock-flag" title="${esc(t('common.mock_hint'))}">${esc(t('common.mock'))}</span>` : ''}${v.actions ? v.actions() : ''}${v.noScanBtn ? '' : scanBtn}</div>`;
}

// ------------------------------------------------------------ layers
export function toast(msg, bad = false) {
  const d = document.createElement('div');
  d.className = 'toast' + (bad ? ' bad' : '');
  d.textContent = msg;
  els.toasts.appendChild(d);
  setTimeout(() => d.remove(), 3200);
}

let sheetClose = null;
/** Open a modal sheet. `build(api)` returns {title, sub, body, foot}; call api.update() to re-render. */
export function openSheet(build, { center = false, onClose } = {}) {
  closeSheet();
  const prevFocus = document.activeElement;
  const wrap = document.createElement('div');
  els.layer.appendChild(wrap);
  const ctl = {
    update() {
      const s = build(ctl);
      wrap.innerHTML = `<div class="scrim" data-act="close-sheet"></div>
        <div class="sheet ${center ? 'center' : ''}" role="dialog" aria-modal="true" aria-labelledby="sheet-t">
          <div class="sheet-h"><div><h2 id="sheet-t">${esc(s.title)}</h2>${s.sub ? `<p>${s.sub}</p>` : ''}</div>
            <button class="btn icon ghost" data-act="close-sheet" aria-label="${esc(t('common.close'))}">${icon('close')}</button></div>
          <div class="sheet-b">${s.body}</div>
          ${s.foot ? `<div class="sheet-f">${s.foot}</div>` : ''}
        </div>`;
    },
    close: () => closeSheet(),
    el: wrap,
  };
  ctl.update();
  const focusable = wrap.querySelector('.sheet-f .btn') || wrap.querySelector('.sheet .btn');
  focusable && focusable.focus();
  sheetClose = () => { wrap.remove(); sheetClose = null; onClose && onClose(); prevFocus && prevFocus.focus && prevFocus.focus(); };
  return ctl;
}
export function closeSheet() { if (sheetClose) sheetClose(); }

// ------------------------------------------------------------ events
const acts = {
  'toggle-lang': () => setLang(getLang() === 'ar' ? 'en' : 'ar'),
  'toggle-theme': () => setTheme(getTheme() === 'dark' ? 'light' : 'dark'),
  'toggle-rail': () => { S.collapsed = !S.collapsed; store('rail', S.collapsed ? '1' : '0'); els.app.classList.toggle('collapsed', S.collapsed); renderRail(); },
  rescan: () => startScan(),
  'cancel-scan': () => cancelScan(),
  'close-sheet': () => closeSheet(),
  go: (el) => go(el.dataset.view),
  copy: async (el) => {
    const txt = el.dataset.copy;
    try { await navigator.clipboard.writeText(txt); } catch (_) {
      const ta = document.createElement('textarea'); ta.value = txt; document.body.appendChild(ta); ta.select();
      try { document.execCommand('copy'); } catch (__) { /* ignore */ } ta.remove();
    }
    const label = el.querySelector('span');
    if (label) { const old = label.textContent; label.textContent = t('common.copied'); setTimeout(() => { label.textContent = old; }, 1400); }
  },
};
export function registerActs(map) { Object.assign(acts, map); }
for (const v of Object.values(VIEWS)) if (v.acts) Object.assign(acts, v.acts);
Object.assign(acts, cleanupActs);

document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-act]');
  if (!el || el.disabled) return;
  const fn = acts[el.dataset.act];
  if (fn) { if (el.tagName === 'A') e.preventDefault(); fn(el, e); }
});
for (const type of ['input', 'change']) {
  document.addEventListener(type, (e) => {
    const el = e.target.closest(`[data-${type}]`);
    if (!el) return;
    const fn = acts[el.dataset[type]];
    if (fn) fn(el, e);
  });
}
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && sheetClose) { closeSheet(); return; }
  const el = e.target.closest('[data-key-act]');
  if (el && (e.key === 'Enter' || e.key === ' ') && e.target === el) { e.preventDefault(); const fn = acts[el.dataset.keyAct]; fn && fn(el, e); }
});

onLang(() => renderAll());

// ------------------------------------------------------------ boot
async function boot() {
  initLang();
  S.collapsed = read('rail', '0') === '1' || window.innerWidth < 1100;
  els.app.classList.toggle('collapsed', S.collapsed);
  try { S.roots = JSON.parse(read('roots', '[]')) || []; } catch (_) { S.roots = []; }
  const v = location.hash.slice(1);
  go(VIEWS[v] ? v : 'overview');
  try {
    const r = await api.lastReport();
    if (r) adoptReport(r);
  } catch (e) {
    console.warn('last_report failed', e);
  }
  renderAll();
  // A full walk touches millions of files while agents are compiling: only
  // rescan on launch when the saved report is stale (Rescan is one click).
  const age = S.report ? Date.now() / 1000 - S.report.generated_at : Infinity;
  if (age > FRESH_SECS) startScan();
}
const FRESH_SECS = 30 * 60;
boot();

export { api, isMock, fmtBytes };
