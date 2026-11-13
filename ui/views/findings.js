import { S, go, touchSelection } from '../app.js';
import { t, fmtDate, fmtAge, fmtInt } from '../i18n.js';
import { icon, esc, B, L, tilde, midTrunc, safetyBadge, ownerChip, catColor, idleText, isFsPath, toolOf, codeLine, CATS, selectionBytes, fitMenus } from '../ui.js';
import { openExplorerAt } from './explorer.js';

const F = { q: '', cats: new Set(), owners: new Set(), safety: new Set(), sort: 'size', open: new Set(), expanded: new Set(), limit: {}, menu: null, init: false };
const PAGE = 40;

export function setCategoryFilter(cat) { F.cats = new Set([cat]); F.open.add(cat); F.init = true; }
export function revealFindings(ids) {
  const r = S.report; if (!r) return;
  F.q = ''; F.cats.clear(); F.owners.clear(); F.safety.clear();
  for (const f of r.findings) if (ids.includes(f.id)) F.open.add(f.category);
  F.init = true; F.focus = ids[0];
}

function filtered(r) {
  const q = F.q.trim().toLowerCase();
  return r.findings.filter((f) =>
    (!F.cats.size || F.cats.has(f.category)) &&
    (!F.owners.size || F.owners.has(f.owner || '')) &&
    (!F.safety.size || F.safety.has(f.safety)) &&
    (!q || f.title.toLowerCase().includes(q) || f.path.toLowerCase().includes(q) || (f.owner || '').toLowerCase().includes(q)));
}

function sorter() {
  if (F.sort === 'age') return (a, b) => (a.last_activity ?? Infinity) - (b.last_activity ?? Infinity);
  if (F.sort === 'name') return (a, b) => a.title.localeCompare(b.title);
  return (a, b) => b.bytes - a.bytes;
}

function groupsOf(list) {
  const m = new Map();
  for (const f of list) { if (!m.has(f.category)) m.set(f.category, []); m.get(f.category).push(f); }
  const s = sorter();
  return [...m.entries()].map(([cat, items]) => ({ cat, items: items.sort(s), bytes: items.reduce((x, f) => x + f.bytes, 0), rec: items.filter((f) => !f.protected && f.safety !== 'review').reduce((x, f) => x + f.bytes, 0) }))
    .sort((a, b) => b.bytes - a.bytes);
}

function row(f, max, now) {
  const sel = S.selected.has(f.id);
  const exp = F.expanded.has(f.id);
  const p = tilde(f.path);
  const stale = f.last_activity && now - f.last_activity > 86400 * 30;
  const w = Math.max(0.6, (f.bytes / max) * 100);
  return `<div class="frow ${sel ? 'sel' : ''}" data-id="${f.id}">
    <input type="checkbox" class="cb" data-change="fd-pick" data-id="${f.id}" ${sel ? 'checked' : ''} ${f.protected ? 'disabled' : ''} aria-label="${esc(f.title)}">
    <div class="f-main"><div class="f-title"><bdi>${esc(f.title)}</bdi></div>
      ${f.protected ? `<div class="f-lock" title="${esc(f.protected)}">${icon('lock')}${esc(t('fd.protected'))} · <bdi>${esc(f.protected)}</bdi></div>` : `<div class="f-path" title="${esc(f.path)}">${esc(midTrunc(p, 72))}</div>`}</div>
    <div>${ownerChip(f.owner)}</div>
    <div>${safetyBadge(f.safety)}</div>
    <div class="f-size">${B(f.bytes)}<div class="f-bar"><i style="inline-size:${w.toFixed(2)}%;--tone:${catColor(f.category)}"></i></div></div>
    <div class="f-age ${stale ? 'stale' : ''}" title="${f.last_activity ? esc(fmtDate(f.last_activity)) : ''}">${esc(idleText(f.last_activity, now))}</div>
    <button class="f-exp" data-act="fd-exp" data-id="${f.id}" aria-expanded="${exp}" aria-label="${esc(t('fd.why'))}">${icon('chevronDown')}</button>
  </div>${exp ? detail(f) : ''}`;
}

function actionBlock(f) {
  const a = f.action || {};
  if (a.kind === 'delete') return `<h4>${esc(t('fd.will_delete'))}</h4>${(a.paths || []).map((x) => codeLine(x, 'rm')).join('')}`;
  if (a.kind === 'run_tool') return `<h4>${esc(t('fd.will_run'))}</h4>${codeLine([a.program, ...(a.args || [])].join(' '))}`;
  if (a.kind === 'relocate') return `<h4>${esc(t('fd.relocate'))}</h4>${codeLine(a.suggested_root || '')}`;
  return `<h4>${esc(t('fd.action'))}</h4><p>${esc(t('fd.review_only'))}</p>`;
}

function detail(f) {
  const tool = toolOf(f.path);
  return `<div class="fdetail">
    <div><h4>${esc(t('fd.why'))}</h4><p><bdi>${esc(f.reason)}</bdi></p>${actionBlock(f)}
      ${f.relocate_to ? `<h4>${esc(t('fd.relocate'))}</h4>${codeLine(f.relocate_to, '>')}` : ''}</div>
    <div><h4>&nbsp;</h4><dl>
      <dt>${esc(t('fd.files'))}</dt><dd>${L(fmtInt(f.files))}</dd>
      <dt>${esc(t('fd.last'))}</dt><dd>${f.last_activity ? `${esc(fmtAge(f.last_activity))} <span class="faint">(${esc(fmtDate(f.last_activity))})</span>` : esc(t('common.never'))}</dd>
      <dt>${esc(t('fd.owner'))}</dt><dd>${ownerChip(f.owner)}</dd>
      <dt>${esc(t('fd.safety'))}</dt><dd>${esc(t('safety.' + f.safety + '_d'))}</dd>
      <dt>${esc(t('fd.category'))}</dt><dd>${esc(t('cat.' + f.category))} <span class="faint">${L(f.rule || '')}</span></dd>
    </dl>
    <div style="margin-block-start:12px">${isFsPath(f.path) ? `<button class="btn sm" data-act="fd-explore" data-path="${esc(f.path)}">${icon('explorer')}${esc(t('common.open_explorer'))}</button>` : tool ? `<span class="faint">${esc(t('fd.managed', { tool }))}</span>` : ''}</div></div>
  </div>`;
}

function groupHtml(g, max, now) {
  const open = F.open.has(g.cat);
  const pickable = g.items.filter((f) => !f.protected);
  const nSel = pickable.filter((f) => S.selected.has(f.id)).length;
  const lim = F.limit[g.cat] || PAGE;
  const shown = g.items.slice(0, lim);
  return `<div class="group ${open ? 'open' : ''}" data-cat="${g.cat}">
    <div class="group-h" data-act="fd-toggle" data-key-act="fd-toggle" data-cat="${g.cat}" tabindex="0" role="button" aria-expanded="${open}">
      ${icon('chevron', 'tw')}
      <input type="checkbox" class="cb" data-change="fd-pick-group" data-cat="${g.cat}" ${nSel && nSel === pickable.length ? 'checked' : ''} ${pickable.length ? '' : 'disabled'} aria-label="${esc(t('fd.select_group'))}" data-ind="${nSel > 0 && nSel < pickable.length ? 1 : 0}">
      <i class="sw" style="background:${catColor(g.cat)}"></i>
      <div class="g-name">${esc(t('cat.' + g.cat))}<span class="g-n">${esc(t('common.items', { count: g.items.length }))}</span></div>
      <span class="g-rec">${g.rec ? B(g.rec) : ''}</span>
      <span class="g-tot">${B(g.bytes)}</span>
    </div>
    ${open ? `<div class="rows">${shown.map((f) => row(f, max, now)).join('')}
      ${g.items.length > lim ? `<div class="more-row"><button class="btn sm ghost" data-act="fd-more" data-cat="${g.cat}">${esc(t('common.show_more', { count: Math.min(PAGE * 3, g.items.length - lim) }))}</button></div>` : ''}</div>` : ''}
  </div>`;
}

function menuHtml(kind, r) {
  const counts = new Map();
  for (const f of r.findings) { const k = kind === 'cat' ? f.category : (f.owner || ''); counts.set(k, (counts.get(k) || 0) + 1); }
  const keys = kind === 'cat' ? CATS.filter((c) => counts.has(c)) : [...counts.keys()].sort((a, b) => counts.get(b) - counts.get(a));
  const set = kind === 'cat' ? F.cats : F.owners;
  return `<div class="menu" role="menu">${keys.map((k) => `<label><input type="checkbox" class="cb" data-change="fd-filter" data-kind="${kind}" data-k="${esc(k)}" ${set.has(k) ? 'checked' : ''}>
    ${kind === 'cat' ? `<i class="sw" style="background:${catColor(k)}"></i>${esc(t('cat.' + k))}` : ownerChip(k || null)}<span class="m-n">${esc(fmtInt(counts.get(k)))}</span></label>`).join('')}</div>`;
}

function toolsHtml(r) {
  const any = F.q || F.cats.size || F.owners.size || F.safety.size;
  const sc = { safe: 0, rebuild: 0, review: 0 };
  r.findings.forEach((f) => { sc[f.safety]++; });
  return `<div class="fd-tools">
    <label class="search">${icon('search')}<span class="sr">${esc(t('fd.search'))}</span><input class="input" type="search" placeholder="${esc(t('fd.search'))}" value="${esc(F.q)}" data-input="fd-q"></label>
    ${['safe', 'rebuild', 'review'].map((s) => `<button class="chip" aria-pressed="${F.safety.has(s)}" data-act="fd-safety" data-s="${s}" title="${esc(t('safety.' + s + '_d'))}">${icon(s)}${esc(t('safety.' + s))}<span class="chip-n">${esc(fmtInt(sc[s]))}</span></button>`).join('')}
    <div class="menu-wrap"><button class="chip" aria-pressed="${F.cats.size > 0}" aria-expanded="${F.menu === 'cat'}" data-act="fd-menu" data-menu="cat">${icon('filter')}${esc(t('fd.category'))}${F.cats.size ? `<span class="chip-n">${esc(fmtInt(F.cats.size))}</span>` : ''}</button>${F.menu === 'cat' ? menuHtml('cat', r) : ''}</div>
    <div class="menu-wrap"><button class="chip" aria-pressed="${F.owners.size > 0}" aria-expanded="${F.menu === 'owner'}" data-act="fd-menu" data-menu="owner">${icon('agents')}${esc(t('fd.owner'))}${F.owners.size ? `<span class="chip-n">${esc(fmtInt(F.owners.size))}</span>` : ''}</button>${F.menu === 'owner' ? menuHtml('owner', r) : ''}</div>
    ${any ? `<button class="btn sm ghost" data-act="fd-clear">${icon('close')}${esc(t('fd.clear_filters'))}</button>` : ''}
    <span class="spacer"></span>
    <div class="seg" role="group" aria-label="${esc(t('fd.sort'))}">${['size', 'age', 'name'].map((k) => `<button aria-pressed="${F.sort === k}" data-act="fd-sort" data-k="${k}">${esc(t('fd.sort_' + k))}</button>`).join('')}</div>
  </div>`;
}

function barHtml(r) {
  const sel = r.findings.filter((f) => S.selected.has(f.id));
  if (!sel.length) return '';
  const total = selectionBytes(sel);
  const mix = { safe: 0, rebuild: 0, review: 0 };
  sel.forEach((f) => { mix[f.safety] += f.bytes; });
  const movable = sel.some((f) => f.relocate_to);
  return `<div class="actionbar" role="region" aria-label="${esc(t('fd.preview'))}">
    <span class="ab-n">${esc(t('fd.selected', { count: sel.length }))}</span><span class="ab-b">${B(total)}</span>
    <span class="ab-mix" aria-hidden="true">${['safe', 'rebuild', 'review'].map((k) => `<i style="inline-size:${((mix[k] / total) * 100).toFixed(1)}%;background:var(--${k})"></i>`).join('')}</span>
    <span class="spacer"></span>
    <button class="btn ghost" data-act="fd-none">${esc(t('common.clear'))}</button>
    ${movable ? `<button class="btn" data-act="clean-selected" data-mode="relocate">${icon('move')}${esc(t('fd.move'))}</button>` : ''}
    <button class="btn primary" data-act="clean-selected" data-mode="reclaim">${icon('trash')}${esc(t('fd.preview'))}</button>
  </div>`;
}

function groupsHtml(r) {
  const list = filtered(r);
  if (!list.length) return `<div class="empty"><h3>${esc(t('fd.empty'))}</h3><button class="btn" data-act="fd-clear">${esc(t('fd.clear_filters'))}</button></div>`;
  const gs = groupsOf(list);
  if (!F.init) { F.open.add(gs[0].cat); F.init = true; }
  const max = Math.max(...list.map((f) => f.bytes));
  const now = Date.now() / 1000;
  return gs.map((g) => groupHtml(g, max, now)).join('');
}

function paint() {
  const r = S.report; if (!r) return;
  const g = document.getElementById('fd-groups');
  if (g) { g.innerHTML = groupsHtml(r); applyIndeterminate(g); }
  const b = document.getElementById('fd-bar');
  if (b) b.innerHTML = barHtml(r);
}
function applyIndeterminate(root) { root.querySelectorAll('[data-ind="1"]').forEach((c) => { c.indeterminate = true; }); }
function full() { const v = document.getElementById('view'); view.render(v); fitMenus(v); }

const view = {
  title: () => t('fd.title'),
  sub: () => (S.report ? `${esc(t('common.items', { count: S.report.findings.length }))} · ${B(S.report.findings.reduce((s, f) => s + f.bytes, 0))}` : ''),
  actions: () => (S.report ? `<button class="btn ghost" data-act="fd-preselect">${esc(t('fd.preselect'))}</button>` : ''),
  render(el) {
    const r = S.report;
    if (!r) { el.innerHTML = `<div class="groups">${'<div class="group"><div class="group-h"><span></span><span></span><span></span><div class="skel" style="block-size:14px;inline-size:40%"></div></div></div>'.repeat(5)}</div>`; return; }
    el.innerHTML = `${toolsHtml(r)}<div class="groups" id="fd-groups">${groupsHtml(r)}</div><div id="fd-bar">${barHtml(r)}</div>`;
    el.classList.add('has-bar');
    applyIndeterminate(el);
    if (F.focus) {
      const id = F.focus; F.focus = null;
      requestAnimationFrame(() => { const row = el.querySelector(`.frow[data-id="${id}"]`); if (row) { row.scrollIntoView({ block: 'center' }); row.classList.add('flash'); } });
    }
  },
  mount() { document.addEventListener('click', outside, true); },
  unmount() { document.removeEventListener('click', outside, true); F.menu = null; },
  acts: {
    'fd-q': (el) => { F.q = el.value; paint(); },
    'fd-safety': (el) => { const s = el.dataset.s; F.safety.has(s) ? F.safety.delete(s) : F.safety.add(s); full(); },
    'fd-menu': (el) => { F.menu = F.menu === el.dataset.menu ? null : el.dataset.menu; full(); },
    'fd-filter': (el) => { const set = el.dataset.kind === 'cat' ? F.cats : F.owners; el.checked ? set.add(el.dataset.k) : set.delete(el.dataset.k); if (el.dataset.kind === 'cat' && el.checked) F.open.add(el.dataset.k); full(); },
    'fd-clear': () => { F.q = ''; F.cats.clear(); F.owners.clear(); F.safety.clear(); F.menu = null; full(); },
    'fd-sort': (el) => { F.sort = el.dataset.k; full(); },
    'fd-toggle': (el, e) => { if (e && e.target.closest('input')) return; const c = el.dataset.cat; F.open.has(c) ? F.open.delete(c) : F.open.add(c); paint(); },
    'fd-more': (el) => { const c = el.dataset.cat; F.limit[c] = (F.limit[c] || PAGE) + PAGE * 3; paint(); },
    'fd-exp': (el) => { const id = el.dataset.id; F.expanded.has(id) ? F.expanded.delete(id) : F.expanded.add(id); paint(); },
    'fd-pick': (el) => { touchSelection(); el.checked ? S.selected.add(el.dataset.id) : S.selected.delete(el.dataset.id); paint(); },
    'fd-pick-group': (el) => {
      touchSelection();
      const items = filtered(S.report).filter((f) => f.category === el.dataset.cat && !f.protected);
      items.forEach((f) => (el.checked ? S.selected.add(f.id) : S.selected.delete(f.id)));
      paint();
    },
    'fd-none': () => { touchSelection(); S.selected.clear(); paint(); },
    'fd-preselect': () => { touchSelection(); S.selected = new Set(S.report.findings.filter((f) => f.preselected && !f.protected).map((f) => f.id)); full(); },
    'fd-explore': (el) => openExplorerAt(el.dataset.path),
  },
};

function outside(e) {
  if (!F.menu) return;
  if (e.target.closest('.menu-wrap')) return;
  F.menu = null; full();
}

export default view;
