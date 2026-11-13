// Disk use over time for the system disk, from every saved scan (≤ 30 days).
// Hand-drawn SVG: one 2px series, a dashed "full" ceiling, recessive grid,
// a crosshair + tooltip on hover, and a screen-reader table of the points.
import { S } from '../app.js';
import { api } from '../api.js';
import { t, fmtBytes, fmtDate } from '../i18n.js';
import { esc, B } from '../ui.js';

const H = { points: null, forReport: 0, loading: false, ro: null, drawnW: 0 };
const GB = 2 ** 30;

export function trendPanel(r) {
  if (H.forReport !== r.generated_at && !H.loading) load(r);
  return `<div class="section-t">${esc(t('tr.title'))}<span class="st-sub">${esc(t('tr.sub', { mount: rootMount(r) }))}</span></div>
    <div class="panel trend" id="trend">${body(r)}</div>`;
}

function rootMount(r) { return (r.disks.find((d) => d.mount === '/') || r.disks.find((d) => d.mount) || {}).mount || '/'; }

async function load(r) {
  H.loading = true;
  try { H.points = await api.getHistory(); } catch (_) { H.points = []; }
  H.loading = false; H.forReport = r.generated_at;
  const el = document.getElementById('trend');
  if (el) { el.innerHTML = body(r); draw(r); }
}

function series(r) {
  const m = rootMount(r);
  const pts = (H.points || []).filter((p) => p.disks && p.disks[m] != null).map((p) => ({ at: p.at, used: p.disks[m] }));
  const disk = r.disks.find((d) => d.mount === m);
  // The current report is the newest point even if history was written a moment earlier.
  if (disk && !pts.some((p) => p.at === r.generated_at)) pts.push({ at: r.generated_at, used: disk.used });
  pts.sort((a, b) => a.at - b.at);
  return { pts, disk };
}

function body(r) {
  if (!H.points) return '<div class="skel" style="block-size:180px"></div>';
  const { pts, disk } = series(r);
  if (pts.length < 3 || !disk) return `<p class="faint tr-empty">${esc(t('tr.empty'))}</p>`;
  const first = pts[0], last = pts[pts.length - 1];
  const week = pts.find((p) => p.at >= last.at - 7 * 86400) || first;
  const d7 = last.used - week.used;
  const g = r.growth?.disks?.find((x) => x.mount === rootMount(r));
  const eta = g && g.days_to_full != null && g.per_day > 0 ? t('tr.full_in', { count: Math.max(1, Math.round(g.days_to_full)) }) : t('tr.not_filling');
  const rows = pts.map((p) => `<tr><td>${esc(fmtDate(p.at))}</td><td>${esc(fmtBytes(p.used))}</td></tr>`).join('');
  return `<div class="tr-stats">
      <div><span class="kicker">${esc(t('tr.now'))}</span><b>${B(last.used)}</b><span class="faint">${esc(t('tr.of', { size: fmtBytes(disk.total) }))}</span></div>
      <div><span class="kicker">${esc(t('tr.week'))}</span><b class="${d7 > 0 ? 'up' : 'down'}"><bdi class="num" dir="ltr">${d7 >= 0 ? '+' : '−'}${esc(fmtBytes(Math.abs(d7)))}</bdi></b></div>
      <div><span class="kicker">${esc(t('tr.outlook'))}</span><b>${esc(eta)}</b></div>
    </div>
    <div class="tr-plot" id="tr-plot" role="img" aria-label="${esc(t('tr.aria', { from: fmtBytes(first.used), to: fmtBytes(last.used), n: pts.length }))}"></div>
    <table class="sr"><caption>${esc(t('tr.title'))}</caption><tbody>${rows}</tbody></table>`;
}

export function draw(r = S.report) {
  const box = document.getElementById('tr-plot');
  if (!box || !r) return;
  // Redraw on real width changes only: observing fires once immediately, and
  // redrawing from that callback unconditionally would loop every frame.
  if (!H.ro) {
    H.ro = new ResizeObserver(() => {
      const b = document.getElementById('tr-plot');
      if (b && S.report && Math.abs(b.clientWidth - H.drawnW) > 2) draw(S.report);
    });
  }
  H.ro.disconnect(); H.ro.observe(box);
  H.drawnW = box.clientWidth;
  const { pts, disk } = series(r);
  if (pts.length < 3) return;
  const W = Math.max(280, box.clientWidth), Hh = 190;
  const pad = { l: 8, r: 64, t: 14, b: 26 };
  const cap = disk.used + disk.available;
  const lo = Math.max(0, Math.min(...pts.map((p) => p.used)) - 0.08 * cap);
  const x0 = pts[0].at, x1 = Math.max(pts[pts.length - 1].at, x0 + 3600);
  const X = (at) => pad.l + ((at - x0) / (x1 - x0)) * (W - pad.l - pad.r);
  const Y = (b) => pad.t + (1 - (b - lo) / (cap - lo)) * (Hh - pad.t - pad.b);
  const step = niceStep((cap - lo) / GB / 3) * GB;
  const grid = [];
  for (let v = Math.ceil(lo / step) * step; v < cap - step * 0.3; v += step) grid.push(v);
  const days = Math.max(1, (x1 - x0) / 86400);
  const tickEvery = days > 20 ? 7 : days > 6 ? 2 : 1;
  const ticks = [];
  for (let d = Math.ceil(x0 / 86400 / tickEvery) * tickEvery; d * 86400 <= x1; d += tickEvery) ticks.push(d * 86400);
  const line = pts.map((p, i) => `${i ? 'L' : 'M'}${X(p.at).toFixed(1)},${Y(p.used).toFixed(1)}`).join('');
  const area = `${line}L${X(pts[pts.length - 1].at).toFixed(1)},${Y(lo).toFixed(1)}L${X(pts[0].at).toFixed(1)},${Y(lo).toFixed(1)}Z`;
  // Label only big drops (cleanups) and the latest value: selective, not every point.
  const drops = [];
  for (let i = 1; i < pts.length; i++) { const d = pts[i - 1].used - pts[i].used; if (d > 20 * GB) drops.push({ p: pts[i], d }); }
  const last = pts[pts.length - 1];
  const fmtDay = new Intl.DateTimeFormat(document.documentElement.lang === 'ar' ? 'ar-u-nu-latn' : 'en', { month: 'short', day: 'numeric' });
  box.innerHTML = `<svg width="${W}" height="${Hh}" viewBox="0 0 ${W} ${Hh}" aria-hidden="true">
    <defs><linearGradient id="trg" x1="0" x2="0" y1="0" y2="1"><stop offset="0" stop-color="var(--accent)" stop-opacity=".16"/><stop offset="1" stop-color="var(--accent)" stop-opacity="0"/></linearGradient></defs>
    ${grid.map((v) => `<line class="tr-grid" x1="${pad.l}" x2="${W - pad.r}" y1="${Y(v).toFixed(1)}" y2="${Y(v).toFixed(1)}"/><text class="tr-ax" x="${W - pad.r + 8}" y="${(Y(v) + 4).toFixed(1)}">${esc(fmtBytes(v))}</text>`).join('')}
    <line class="tr-cap" x1="${pad.l}" x2="${W - pad.r}" y1="${Y(cap).toFixed(1)}" y2="${Y(cap).toFixed(1)}"/>
    <text class="tr-ax strong" x="${W - pad.r + 8}" y="${(Y(cap) + 4).toFixed(1)}">${esc(t('tr.full'))}</text>
    ${ticks.map((at) => `<text class="tr-ax" text-anchor="middle" x="${X(at).toFixed(1)}" y="${Hh - 8}">${esc(fmtDay.format(new Date(at * 1000)))}</text>`).join('')}
    <path d="${area}" fill="url(#trg)"/>
    <path class="tr-line" d="${line}"/>
    ${drops.map(({ p, d }) => `<text class="tr-note" text-anchor="middle" x="${X(p.at).toFixed(1)}" y="${(Y(p.used) + 18).toFixed(1)}">−${esc(fmtBytes(d))}</text>`).join('')}
    <circle class="tr-last" cx="${X(last.at).toFixed(1)}" cy="${Y(last.used).toFixed(1)}" r="4"/>
    <g class="tr-hover" style="display:none"><line class="tr-cross" y1="${pad.t}" y2="${Hh - pad.b}"/><circle class="tr-dot" r="4.5"/></g>
    <rect class="tr-hit" x="${pad.l}" y="0" width="${W - pad.l - pad.r}" height="${Hh}" fill="transparent"/>
  </svg><div class="tr-tip" role="status" style="display:none"></div>`;
  const svg = box.querySelector('svg'), hov = svg.querySelector('.tr-hover'), tip = box.querySelector('.tr-tip');
  const hit = svg.querySelector('.tr-hit');
  hit.onmousemove = (e) => {
    const mx = e.clientX - svg.getBoundingClientRect().left;
    let best = pts[0];
    for (const p of pts) if (Math.abs(X(p.at) - mx) < Math.abs(X(best.at) - mx)) best = p;
    const cx = X(best.at), cy = Y(best.used);
    hov.style.display = '';
    hov.querySelector('.tr-cross').setAttribute('x1', cx); hov.querySelector('.tr-cross').setAttribute('x2', cx);
    const dot = hov.querySelector('.tr-dot'); dot.setAttribute('cx', cx); dot.setAttribute('cy', cy);
    tip.style.display = '';
    tip.innerHTML = `<div class="faint">${esc(fmtDate(best.at))}</div><div>${B(best.used)} <span class="faint">${esc(t('tr.used'))}</span></div><div>${B(cap - best.used)} <span class="faint">${esc(t('tr.free'))}</span></div>`;
    const tw = tip.offsetWidth;
    tip.style.insetInlineStart = 'auto';
    tip.style.left = `${Math.min(Math.max(0, cx + 12), W - tw - 4)}px`;
    tip.style.top = `${Math.max(0, cy - 60)}px`;
  };
  hit.onmouseleave = () => { hov.style.display = 'none'; tip.style.display = 'none'; };
}

function niceStep(x) {
  const p = 10 ** Math.floor(Math.log10(x)); const f = x / p;
  return (f < 1.5 ? 1 : f < 3.5 ? 2 : f < 7.5 ? 5 : 10) * p;
}
