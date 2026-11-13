import { S, go } from '../app.js';
import { api, onProgress } from '../api.js';
import { t, fmtCompact, fmtInt, fmtAge, fmtBytes } from '../i18n.js';
import { icon, esc, B, L, tilde, home, catColor } from '../ui.js';

const X = { root: null, tree: null, stack: [], loading: false, files: 0, bytes: 0, error: null, ro: null, want: null, cache: new Map() };

function roots() {
  const out = [];
  const h = home();
  if (h) out.push({ path: h, label: t('ex.home') });
  for (const d of (S.report?.disks || [])) if (d.mount && d.mount !== '/boot/efi') out.push({ path: d.mount, label: d.mount });
  return out;
}

export function openExplorerAt(path) {
  const rs = roots();
  const r = rs.filter((x) => path === x.path || path.startsWith(x.path.replace(/\/$/, '') + '/')).sort((a, b) => b.path.length - a.path.length)[0];
  if (!r) return;
  X.want = path;
  if (X.root !== r.path || !X.tree) { X.root = r.path; X.tree = null; X.stack = []; }
  else { X.stack = [X.tree]; drillWant(); }
  go('explorer');
}

async function load(path) {
  if (X.cache.has(path)) { X.root = path; X.tree = X.cache.get(path); X.stack = [X.tree]; drillWant(); paint(); return; }
  X.root = path; X.loading = true; X.error = null; X.files = 0; X.bytes = 0; X.tree = null; X.stack = [];
  paint();
  const off = onProgress((p) => {
    X.files = p.files; X.bytes = p.bytes;
    const n = document.getElementById('ex-prog');
    if (n) n.innerHTML = progText();
  });
  try {
    const tree = await api.getTree(path);
    X.cache.set(path, tree);
    if (X.root === path) { X.tree = tree; X.stack = [tree]; drillWant(); }
  } catch (e) {
    X.error = String(e?.message || e);
  } finally {
    off(); X.loading = false; paint();
  }
}

function drillWant() {
  if (!X.want || !X.tree) return;
  const rel = X.want.slice(X.root.replace(/\/$/, '').length + 1).split('/').filter(Boolean);
  let n = X.tree;
  for (const part of rel) {
    const c = n.children.find((x) => x.name === part);
    if (!c || !c.children.length) break;
    X.stack.push(c); n = c;
  }
  X.want = null;
}

const progText = () => esc(t('ex.walked', { files: fmtCompact(X.files), bytes: fmtBytes(X.bytes) }));

function pathOf(depth) {
  const base = X.root.replace(/\/$/, '');
  return [base, ...X.stack.slice(1, depth + 1).map((n) => n.name)].join('/') || '/';
}

function tintFor(path, bytes) {
  const by = {};
  let tot = 0;
  for (const f of S.report?.findings || []) {
    if (path === f.path || path.startsWith(f.path + '/')) return { cat: f.category, frac: 1 };
    if (f.path.startsWith(path + '/')) { by[f.category] = (by[f.category] || 0) + f.bytes; tot += f.bytes; }
  }
  if (!tot || !bytes) return null;
  const [cat, b] = Object.entries(by).sort((a, c) => c[1] - a[1])[0];
  const frac = Math.min(1, b / bytes);
  return frac > 0.25 ? { cat, frac } : null;
}

function squarify(items, W, H) {
  const total = items.reduce((s, i) => s + i.v, 0) || 1;
  const scale = (W * H) / total;
  const its = items.map((i) => ({ ...i, a: i.v * scale })).filter((i) => i.a > 1);
  const res = [];
  let x = 0, y = 0, w = W, h = H, k = 0;
  const worst = (row, side) => { const s = row.reduce((a, r) => a + r.a, 0); const mx = Math.max(...row.map((r) => r.a)), mn = Math.min(...row.map((r) => r.a)); return Math.max((side * side * mx) / (s * s), (s * s) / (side * side * mn)); };
  while (k < its.length && w > 0.5 && h > 0.5) {
    const side = Math.min(w, h);
    let row = [its[k++]];
    while (k < its.length) { const nr = row.concat(its[k]); if (worst(nr, side) <= worst(row, side)) { row = nr; k++; } else break; }
    const s = row.reduce((a, r) => a + r.a, 0); const th = s / side;
    if (w >= h) { let yy = y; for (const r of row) { const hh = r.a / th; res.push({ ...r, x, y: yy, w: th, h: hh }); yy += hh; } x += th; w -= th; }
    else { let xx = x; for (const r of row) { const ww = r.a / th; res.push({ ...r, x: xx, y, w: ww, h: th }); xx += ww; } y += th; h -= th; }
  }
  return res;
}

function layout() {
  const box = document.getElementById('tmap');
  const node = X.stack[X.stack.length - 1];
  if (!box || !node) return;
  const { width, height } = box.getBoundingClientRect();
  const items = node.children.slice(0, 80).map((c, i) => ({ v: c.bytes, i, node: c }));
  if (node.other_bytes > 0) items.push({ v: node.other_bytes, other: true });
  if (!items.length) { box.innerHTML = `<div class="ex-loading"><span>${esc(t('ex.empty'))}</span></div>`; return; }
  items.sort((a, b) => b.v - a.v);
  const cells = squarify(items, width, height);
  const base = pathOf(X.stack.length - 1);
  box.innerHTML = cells.map((c) => {
    // Name + size need ~46px; name alone ~32px (7px padding each side + a 16px line). Smaller cells keep only the tooltip.
    const big = c.w > 64 && c.h >= 46;
    const mid = !big && c.w > 52 && c.h >= 32;
    const st = `left:${c.x.toFixed(1)}px;top:${c.y.toFixed(1)}px;width:${c.w.toFixed(1)}px;height:${c.h.toFixed(1)}px`;
    if (c.other) return `<div class="tcell other" style="${st}" title="${esc(t('ex.other'))} · ${esc(fmtBytes(c.v))}">${big ? `<div class="tc-in"><span class="tc-n" style="font-family:var(--font)">${esc(t('ex.other'))}</span><span class="tc-b">${esc(fmtBytes(c.v))}</span></div>` : ''}</div>`;
    const p = base.replace(/\/$/, '') + '/' + c.node.name;
    const f = tintFor(p, c.v);
    const leaf = !c.node.children.length;
    return `<button class="tcell ${f ? 'tinted' : ''} ${leaf ? 'leaf' : ''}" style="${st};${f ? `--tone:${catColor(f.cat)};--mix:${Math.round(9 + f.frac * 15)}%` : ''}" ${leaf ? 'tabindex="-1"' : `data-act="ex-drill" data-i="${c.i}"`} title="${esc(p)} · ${esc(fmtBytes(c.v))}${f ? ' · ' + esc(t('cat.' + f.cat)) : ''}">
      ${big ? `<span class="tc-in"><span class="tc-n">${esc(c.node.name)}</span><span class="tc-b">${esc(fmtBytes(c.v))}</span></span>` : mid ? `<span class="tc-in"><span class="tc-n">${esc(c.node.name)}</span></span>` : ''}</button>`;
  }).join('');
}

function side(node) {
  const tf = node.top_files || [];
  return `<div class="panel"><div class="panel-h"><h2>${esc(t('ex.details'))}</h2></div><div class="panel-b"><dl>
      <dt>${esc(t('ex.size'))}</dt><dd>${B(node.bytes)}</dd>
      <dt>${esc(t('ex.files'))}</dt><dd>${L(fmtInt(node.files))}</dd>
      <dt>${esc(t('ex.dirs'))}</dt><dd>${L(fmtInt(node.dirs))}</dd>
      <dt>${esc(t('ex.newest'))}</dt><dd style="font-family:var(--font)">${esc(fmtAge(node.newest))}</dd></dl></div></div>
    <div class="panel"><div class="panel-h"><h2>${esc(t('ex.top_files'))}</h2></div><div class="panel-b">
      ${tf.length ? tf.map(([n, b]) => `<div class="tf">${icon('file')}<span title="${esc(n)}">${esc(n)}</span>${B(b)}</div>`).join('') : `<p class="faint" style="font-size:12px">${esc(t('ex.no_files'))}</p>`}
    </div></div>
    <p class="faint" style="font-size:11.5px">${esc(t('ex.tint'))}</p>`;
}

function crumbs() {
  const parts = X.stack.map((n, i) => (i === 0 ? tilde(X.root) : n.name));
  return `<nav class="crumbs" aria-label="path">${parts.map((p, i) => `${i ? '<span class="sep">/</span>' : ''}<button data-act="ex-crumb" data-d="${i}">${esc(p)}</button>`).join('')}</nav>`;
}

function paint() {
  if (S.view !== 'explorer') return;
  const el = document.getElementById('view');
  view.render(el);
}

const view = {
  title: () => t('ex.title'),
  sub: () => esc(t('ex.sub')),
  noScanBtn: true,
  render(el) {
    const rs = roots();
    if (!X.root && rs.length && !X.loading) { X.root = rs[0].path; }
    if (X.root && !X.tree && !X.loading && !X.error && S.view === 'explorer') queueMicrotask(() => load(X.root));
    const node = X.stack[X.stack.length - 1];
    const tools = `<div class="ex-tools"><span class="faint">${esc(t('ex.root'))}</span>
      <div class="seg" role="group">${rs.map((r) => `<button aria-pressed="${X.root === r.path}" data-act="ex-root" data-path="${esc(r.path)}">${r.label === t('ex.home') ? esc(r.label) : L(r.label)}</button>`).join('')}</div>
      ${X.stack.length > 1 ? `<button class="btn sm ghost" data-act="ex-up">${icon('up')}${esc(t('ex.up'))}</button>` : ''}
      ${node ? crumbs() : ''}</div>`;
    let main;
    if (X.loading) main = `<div class="tmap"><div class="ex-loading"><div class="pulse"></div><div>${esc(t('ex.loading', { path: tilde(X.root) }))}</div><div class="stat-v" id="ex-prog">${progText()}</div></div></div><div class="ex-side"><div class="panel"><div class="skel" style="block-size:140px"></div></div></div>`;
    else if (X.error) main = `<div><div class="err">${icon('critical')}<div><b>${esc(t('common.error'))}</b><div class="muted">${esc(X.error)}</div></div></div><button class="btn" style="margin-block-start:10px" data-act="ex-reload">${esc(t('common.retry'))}</button></div><div></div>`;
    else if (!node) main = `<div class="tmap"><div class="ex-loading"><button class="btn primary" data-act="ex-reload">${icon('explorer')}${esc(t('ex.load'))}</button></div></div><div></div>`;
    else main = `<div class="tmap" id="tmap"></div><div class="ex-side">${side(node)}</div>`;
    el.innerHTML = tools + `<div class="ex-grid">${main}</div>`;
    if (node && !X.loading) requestAnimationFrame(layout);
  },
  mount() {
    requestAnimationFrame(layout);
    X.ro = new ResizeObserver(() => layout());
    X.ro.observe(document.getElementById('view'));
  },
  unmount() { if (X.ro) X.ro.disconnect(); X.ro = null; },
  acts: {
    'ex-root': (el) => load(el.dataset.path),
    'ex-reload': () => { X.cache.delete(X.root); load(X.root); },
    'ex-drill': (el) => { const node = X.stack[X.stack.length - 1]; const c = node.children[Number(el.dataset.i)]; if (c && c.children.length) { X.stack.push(c); paint(); } },
    'ex-crumb': (el) => { X.stack = X.stack.slice(0, Number(el.dataset.d) + 1); paint(); },
    'ex-up': () => { if (X.stack.length > 1) { X.stack.pop(); paint(); } },
  },
};
export default view;
