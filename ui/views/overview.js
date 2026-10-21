import { S, go, touchSelection } from '../app.js';
import { t, fmtPct, fmtBytes, fmtAge } from '../i18n.js';
import { icon, esc, B, L, CATS, catColor, rich, codeLine, selectionBytes } from '../ui.js';
import { openCleanup } from './cleanup.js';
import { setCategoryFilter } from './findings.js';

const SEV = { critical: 0, warn: 1, info: 2 };
const ROW = 30;

function isIdleHdd(d) { return !d.mount && d.rotational && d.total > 50 * 2 ** 30; }

function preselected(r) { return r.findings.filter((f) => f.preselected && !f.protected); }

function rootPanel(r) {
  const d = r.disks.find((x) => x.mount === '/') || r.disks.find((x) => x.mount);
  if (!d) return '';
  const used = d.used, total = d.total, free = d.available;
  const reclaim = r.totals.reduce((s, x) => s + x.reclaimable, 0);
  const pctUsed = used / (used + free || total);
  const cap = used + free || total;
  const recW = Math.min(reclaim, used) / cap;
  const restW = Math.max(0, used / cap - recW);
  return `<div class="panel ov-root fade-in">
    <div class="kicker">${icon(d.rotational ? 'hdd' : 'ssd')}${esc(t('ov.root_k'))} ${L(d.mount)} · ${L(d.device)}</div>
    <div class="big">${esc(fmtPct(pctUsed))}<small>${esc(t('ov.used_of', { used: fmtBytes(used), total: fmtBytes(total) }))}</small></div>
    <div class="cap" role="img" aria-label="${esc(fmtPct(pctUsed))}">
      <i style="inline-size:${(restW * 100).toFixed(2)}%;background:var(--text-3)"></i>
      <i style="inline-size:${(recW * 100).toFixed(2)}%;background:var(--accent)"></i>
    </div>
    <div class="cap-legend">
      <span><i class="sw" style="background:var(--text-3)"></i>${esc(t('cat.other'))}<b>${B(used - Math.min(reclaim, used))}</b></span>
      <span><i class="sw" style="background:var(--accent)"></i>${esc(t('ov.reclaimable'))}<b>${B(reclaim)}</b></span>
      <span><i class="sw" style="background:var(--surface-3)"></i>${esc(t('cat.free'))}<b>${B(free)}</b></span>
    </div>
  </div>`;
}

function reclaimPanel(r) {
  const pre = preselected(r);
  const total = selectionBytes(pre);
  const bySafety = { safe: 0, rebuild: 0, review: 0 };
  pre.forEach((f) => { bySafety[f.safety] += f.bytes; });
  return `<div class="panel ov-reclaim fade-in" style="animation-delay:.05s">
    <div class="kicker">${icon('trash')}${esc(t('ov.reclaim_k'))}</div>
    <div class="big">${B(total)}</div>
    <p>${esc(pre.length ? t('ov.reclaim_b', { count: pre.length }) : t('ov.reclaim_none'))}</p>
    <div class="split">${['safe', 'rebuild'].filter((k) => bySafety[k]).map((k) => `<span><i class="sw" style="background:var(--${k})"></i>${esc(t('safety.' + k))} ${B(bySafety[k])}</span>`).join('')}</div>
    <div class="row">
      <button class="btn primary lg" data-act="ov-reclaim" ${pre.length ? '' : 'disabled'}>${icon('arrow')}${esc(t('ov.reclaim_btn'))}</button>
      <button class="btn ghost lg" data-act="go" data-view="findings">${esc(t('ov.see_findings'))}</button>
    </div>
  </div>`;
}

function diskCard(d, r) {
  const name = d.label || d.name;
  const kind = d.rotational ? t('disk.hdd') : t('disk.ssd');
  if (isIdleHdd(d)) {
    const mnt = '/mnt/' + (d.label || d.name).toLowerCase().replace(/[^a-z0-9_-]/g, '');
    const fstab = `UUID=${d.uuid} ${mnt} ${d.fs} defaults,nofail 0 2`;
    return `<div class="panel disk idle-hdd fade-in">
      <div class="disk-h">${icon('hdd')}<div><div class="disk-name">${L(name)}</div><div class="disk-meta">${L(d.device)} · ${L(d.fs)}</div></div><span class="disk-size">${B(d.total)}</span></div>
        <div class="flagrow"><span class="flag">${esc(t('disk.not_mounted'))}</span>${!d.in_fstab ? `<span class="flag quiet">${esc(t('disk.not_fstab'))}</span>` : ''}<span class="flag quiet">${esc(kind)}</span></div>
      <div class="idle-big">${t('disk.idle', { size: `<span>${B(d.total)}</span>` })}</div>
      <p>${esc(t('disk.hdd_pitch'))}</p>
      <div style="display:grid;gap:6px">${codeLine(`sudo mkdir -p ${mnt} && sudo mount ${d.device} ${mnt}`)}${!d.in_fstab ? codeLine(fstab, 'fstab') : ''}</div>
    </div>`;
  }
  if (!d.mount) {
    return `<div class="panel disk unmounted minor fade-in">
      <div class="disk-h">${icon(d.rotational ? 'hdd' : 'ssd')}<div><div class="disk-name">${L(name)}</div><div class="disk-meta">${L(d.device)} · ${L(d.fs)}</div></div><span class="disk-size">${B(d.total)}</span></div>
      <div class="disk-foot"><span>${esc(t('disk.not_mounted'))}</span><span>${esc(kind)}</span></div>
    </div>`;
  }
  const p = d.used + d.available ? d.used / (d.used + d.available) : 0;
  return `<div class="panel disk fade-in">
    <div class="disk-h">${icon(d.rotational ? 'hdd' : 'ssd')}<div><div class="disk-name">${L(name)}</div><div class="disk-meta">${L(d.mount)} · ${L(d.fs)}</div></div><span class="disk-size">${B(d.total)}</span></div>
    <div class="meter ${p > 0.88 ? 'hot' : p > 0.75 ? 'warm' : ''}"><i style="inline-size:${(p * 100).toFixed(1)}%"></i></div>
    <div class="disk-foot"><span>${esc(t('disk.free', { free: fmtBytes(d.available) }))}</span><span class="num">${esc(fmtPct(p))}</span></div>
  </div>`;
}

function strataLayers(r) {
  const layers = r.totals.filter((x) => x.bytes > 0).sort((a, b) => b.bytes - a.bytes)
    .map((x) => ({ key: x.category, bytes: x.bytes, rec: x.reclaimable, color: catColor(x.category), label: t('cat.' + x.category), cat: true }));
  const root = r.disks.find((d) => d.mount === '/');
  const sum = layers.reduce((s, l) => s + l.bytes, 0);
  let mount = '/';
  if (root && sum <= root.used) {
    layers.push({ key: 'other', bytes: root.used - sum, rec: 0, color: 'var(--c-other)', label: t('cat.other') });
    layers.unshift({ key: 'free', bytes: root.available, rec: 0, color: 'transparent', label: t('cat.free'), free: true });
  } else mount = t('cat.other');
  return { layers, mount };
}

function fitHeights(vals, H, min) {
  const total = vals.reduce((s, v) => s + v, 0) || 1;
  let h = vals.map((v) => Math.max(min, (v / total) * H));
  const excess = h.reduce((s, x) => s + x, 0) - H;
  const slack = h.reduce((s, x) => s + (x - min), 0);
  if (excess > 0 && slack > 0) h = h.map((x) => x - ((x - min) / slack) * excess);
  return h;
}

function strataPanel(r) {
  const { layers, mount } = strataLayers(r);
  const H = layers.length * ROW;
  const W = 112, GAP = 2;
  const hs = fitHeights(layers.map((l) => l.bytes), H - GAP * (layers.length - 1), 5);
  let y = 0;
  const defs = [], rects = [], leaders = [];
  layers.forEach((l, i) => {
    const h = hs[i];
    const mid = y + h / 2;
    const rowMid = i * ROW + ROW / 2;
    if (l.free) {
      rects.push(`<rect data-k="${l.key}" x="0.5" y="${y + 0.5}" width="${W - 1}" height="${Math.max(0, h - 1)}" rx="3" style="fill:none;stroke:var(--line-2);stroke-dasharray:3 3"/>`);
    } else {
      const fr = l.bytes ? Math.min(1, l.rec / l.bytes) : 0;
      const wr = W * fr;
      if (l.cat) defs.push(`<pattern id="h-${l.key}" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="5" height="5" style="fill:${l.color};opacity:var(--hatch-op)"/><rect width="1.6" height="5" style="fill:${l.color}"/></pattern>`);
      rects.push(`<g data-k="${l.key}">${wr > 0 ? `<rect x="0" y="${y}" width="${wr}" height="${h}" style="fill:${l.color}"/>` : ''}${wr < W ? `<rect x="${wr}" y="${y}" width="${W - wr}" height="${h}" style="fill:${l.cat ? `url(#h-${l.key})` : l.color}"/>` : ''}</g>`);
    }
    leaders.push(`<path data-k="${l.key}" d="M0 ${mid.toFixed(1)} C 32 ${mid.toFixed(1)}, 32 ${rowMid}, 64 ${rowMid}"/>`);
    y += h + GAP;
  });
  const rows = layers.map((l) => {
    const sw = l.free ? `<i class="sw" style="border:1px dashed var(--line-2)"></i>` : `<i class="sw" style="background:${l.color}"></i>`;
    const tag = l.cat ? 'button' : 'div';
    return `<${tag} class="stratum ${l.cat ? '' : 'static'}" data-k="${l.key}" ${l.cat ? `data-act="ov-cat" data-cat="${l.key}"` : ''}>${sw}<span class="s-name">${esc(l.label)}</span><span class="s-rec">${l.rec ? B(l.rec) : ''}</span><span class="s-tot">${B(l.bytes)}</span></${tag}>`;
  }).join('');
  return `<div class="panel fade-in" style="animation-delay:.1s">
    <div class="panel-h"><h2>${esc(t('ov.where'))}</h2></div>
    <div class="panel-b" style="padding-block:4px 0"><p class="faint" style="font-size:12px">${t('ov.where_sub', { mount: L(mount) })}</p></div>
    <div class="strata" id="strata">
      <div></div><div></div>
      <div class="strata-head"><span></span><span></span><span>${esc(t('ov.reclaimable'))}</span><span>${esc(t('ov.total'))}</span></div>
      <svg class="core" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" dir="ltr" role="img" aria-label="${esc(t('ov.where'))}"><defs>${defs.join('')}<clipPath id="core-clip"><rect width="${W}" height="${H}" rx="6"/></clipPath></defs><g clip-path="url(#core-clip)">${rects.join('')}</g></svg>
      <svg class="leaders" width="64" height="${H}" viewBox="0 0 64 ${H}" aria-hidden="true">${leaders.join('')}</svg>
      <div class="strata-list">${rows}</div>
    </div>
    <div class="panel-b" style="padding-block-start:0"><div class="cap-legend"><span><i class="sw" style="background:var(--text-2)"></i>${esc(t('ov.reclaimable'))}</span><span><i class="sw hatch" style="--tone:var(--text-2)"></i>${esc(t('ov.kept'))}</span></div></div>
  </div>`;
}

function insightsPanel(r) {
  const list = [...r.insights].sort((a, b) => (SEV[a.severity] ?? 3) - (SEV[b.severity] ?? 3));
  return `<div class="fade-in" style="animation-delay:.15s">
    <div class="section-t" style="margin-block-start:0">${esc(t('ov.insights'))}</div>
    <div class="insights">${list.length ? list.map((x) => `<div class="insight ${x.severity}">${icon(x.severity === 'critical' ? 'critical' : x.severity === 'warn' ? 'warn' : 'info')}
      <h3><span class="sev">${esc(t('severity.' + x.severity))}</span><bdi>${rich(x.title)}</bdi></h3><p><bdi>${rich(x.detail)}</bdi></p></div>`).join('') : `<p class="muted">${esc(t('ov.no_insights'))}</p>`}</div>
  </div>`;
}

function skeleton() {
  return `<div class="ov-head"><div class="panel ov-root"><div class="skel" style="block-size:14px;inline-size:40%"></div><div class="skel" style="block-size:42px;inline-size:60%"></div><div class="skel" style="block-size:12px"></div></div>
    <div class="panel ov-reclaim"><div class="skel" style="block-size:14px;inline-size:30%"></div><div class="skel" style="block-size:42px;inline-size:50%"></div><div class="skel" style="block-size:32px;inline-size:45%"></div></div></div>
    <div class="section-t">${esc(t('ov.disks'))}</div><div class="disks">${'<div class="panel disk"><div class="skel" style="block-size:60px"></div></div>'.repeat(3)}</div>`;
}

export default {
  title: () => t('ov.title'),
  sub: () => (S.report ? `${L(S.report.host)} · ${esc(t('scan.last', { age: fmtAge(S.report.generated_at) }))}` : esc(t('scan.never'))),
  render(el) {
    const r = S.report;
    if (!r) {
      el.innerHTML = S.scan.running ? skeleton() : `<div class="empty"><h3>${esc(t('scan.empty_t'))}</h3><p>${esc(t('scan.empty_b'))}</p><button class="btn primary lg" data-act="rescan">${icon('refresh')}${esc(t('common.scan'))}</button></div>`;
      return;
    }
    const disks = [...r.disks].sort((a, b) => (isIdleHdd(b) - isIdleHdd(a)) || ((b.mount === '/') - (a.mount === '/')) || (!!b.mount - !!a.mount) || b.total - a.total);
    el.innerHTML = `
      <div class="ov-head">${rootPanel(r)}${reclaimPanel(r)}</div>
      <div class="section-t">${esc(t('ov.disks'))}</div>
      <div class="disks ${disks.some(isIdleHdd) ? 'has-idle' : ''}">${disks.map((d) => diskCard(d, r)).join('')}</div>
      <div class="ov-grid">${strataPanel(r)}${insightsPanel(r)}</div>`;
  },
  mount(el) {
    el.addEventListener('mouseover', hover);
    el.addEventListener('mouseout', hover);
  },
  unmount() {
    const el = document.getElementById('view');
    el.removeEventListener('mouseover', hover);
    el.removeEventListener('mouseout', hover);
  },
  acts: {
    'ov-reclaim': () => {
      const ids = preselected(S.report).map((f) => f.id);
      S.selected = new Set(ids); touchSelection();
      openCleanup(ids);
    },
    'ov-cat': (el) => { setCategoryFilter(el.dataset.cat); go('findings'); },
  },
};

function hover(e) {
  const box = document.getElementById('strata');
  if (!box) return;
  const it = e.target.closest('#strata [data-k]');
  box.querySelectorAll('.hl').forEach((x) => x.classList.remove('hl'));
  if (e.type === 'mouseover' && it) {
    const k = it.dataset.k;
    box.classList.add('dim');
    box.querySelectorAll(`[data-k="${k}"]`).forEach((x) => x.classList.add('hl'));
    box.querySelectorAll(`.leaders [data-k="${k}"]`).forEach((x) => { x.style.stroke = 'var(--text-2)'; });
  } else {
    box.classList.remove('dim');
    box.querySelectorAll('.leaders path').forEach((x) => { x.style.stroke = ''; });
  }
}
