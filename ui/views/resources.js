import { S, openSheet, toast, closeSheet } from '../app.js';
import { api } from '../api.js';
import { t, fmtPct, fmtBytes, fmtNum, fmtInt, fmtDur } from '../i18n.js';
import { icon, esc, B, L, ownerChip, tilde, home } from '../ui.js';
import { subscribe, live, refreshNow, HIST } from './live.js';

const R = { sort: 'cpu', dir: -1, q: '', group: true, limit: 60, unsub: null };

function heat(v) {
  const k = Math.max(0, Math.min(1, v / 100));
  return `color-mix(in srgb, var(--crit) ${Math.round(Math.max(0, k - 0.35) / 0.65 * 100)}%, var(--accent))`;
}

function spark(hist) {
  const W = 300, H = 76;
  const pts = hist.map((v, i) => [W - (hist.length - 1 - i) * (W / (HIST - 1)), H - 3 - (Math.min(100, v) / 100) * (H - 6)]);
  if (pts.length < 2) return `<svg class="spark" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" dir="ltr"><line class="gd" x1="0" x2="${W}" y1="${H / 2}" y2="${H / 2}"/></svg>`;
  const line = pts.map((p) => p.map((x) => x.toFixed(1)).join(',')).join(' ');
  const area = `M${pts[0][0].toFixed(1)},${H} L${line.replace(/ /g, ' L')} L${pts[pts.length - 1][0].toFixed(1)},${H} Z`;
  return `<svg class="spark" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" dir="ltr" aria-hidden="true">
    <line class="gd" x1="0" x2="${W}" y1="${H * 0.25}" y2="${H * 0.25}"/><line class="gd" x1="0" x2="${W}" y1="${H * 0.5}" y2="${H * 0.5}"/><line class="gd" x1="0" x2="${W}" y1="${H * 0.75}" y2="${H * 0.75}"/>
    <path class="ar" d="${area}"/><polyline class="ln" points="${line}" vector-effect="non-scaling-stroke"/></svg>`;
}

function arcPath(cx, cy, r, a0, a1) {
  const p = (a) => [cx + r * Math.cos((a * Math.PI) / 180), cy + r * Math.sin((a * Math.PI) / 180)];
  const [x0, y0] = p(a0), [x1, y1] = p(a1);
  return `M${x0.toFixed(2)} ${y0.toFixed(2)} A${r} ${r} 0 ${a1 - a0 > 180 ? 1 : 0} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;
}
function gauge(title, used, total, color, extra) {
  const k = total ? used / total : 0;
  const d = arcPath(100, 92, 72, 150, 390);
  return `<div class="panel gauge"><div class="kicker">${icon(title === 'mem' ? 'memory' : 'refresh')}${esc(t('rs.' + title))}</div>
    <svg viewBox="0 0 200 150" dir="ltr" role="img" aria-label="${esc(fmtPct(k))}">
      <path class="g-tr" d="${d}" pathLength="100"/>
      <path class="g-v" d="${d}" pathLength="100" style="stroke:${color};stroke-dasharray:${(k * 100).toFixed(2)} 100"/>
      <text class="g-num" x="100" y="96" text-anchor="middle">${esc(fmtPct(k))}</text>
      <text class="g-sub" x="100" y="118" text-anchor="middle">${esc(fmtBytes(used))} / ${esc(fmtBytes(total))}</text>
    </svg>${extra || ''}</div>`;
}

function topHtml(s, hist) {
  const cores = s.cpu_cores.length;
  return `<div class="panel rs-cpu">
      <div><div class="kicker">${icon('cpu')}${esc(t('rs.cpu'))}</div><div class="big" style="margin-block-start:8px">${esc(fmtPct(s.cpu_total / 100))}</div>
        <div class="faint" style="font-size:11.5px;margin-block-start:4px">${esc(t('rs.history', { n: fmtInt(Math.round(HIST * 1.5)) }))}</div></div>
      <div>${spark(hist)}</div>
      <div class="cores" title="${esc(t('rs.per_core'))}">${s.cpu_cores.map((c, i) => `<div class="core"><div class="core-bar"><i style="block-size:${c.toFixed(1)}%;--heat:${heat(c)}"></i></div><span>${i}</span></div>`).join('')}</div>
    </div>
    ${gauge('mem', s.mem_used, s.mem_total, 'var(--accent)', `<div class="g-row"><span>${esc(t('rs.avail_k'))}</span><b>${B(s.mem_available)}</b></div>`)}
    ${gauge('swap', s.swap_used, s.swap_total, 'var(--rebuild)', `<div class="g-row"><span>${esc(t('rs.swap'))}</span><b>${B(s.swap_used)}</b></div>`)}`;
  void cores;
}

function row2Html(s) {
  const cores = s.cpu_cores.length;
  const keys = ['load_1', 'load_5', 'load_15'];
  return `<div class="panel tile"><div class="stat-k">${esc(t('rs.load'))}</div>
      <div class="load-v">${s.load.map((v, i) => `<div><span class="stat-v ${v > cores ? 'over' : ''}">${L(fmtNum(v, 2))}</span><span class="stat-k">${esc(t('rs.' + keys[i]))}</span></div>`).join('')}</div>
      <div class="faint" style="font-size:11.5px">${esc(t('rs.load_hint', { cores: fmtInt(cores) }))}</div></div>
    <div class="panel tile"><div class="stat-k">${esc(t('rs.procs'))}</div><div class="stat-v">${L(fmtInt(s.process_count))}</div></div>
    <div class="panel tile"><div class="stat-k">${esc(t('rs.threads'))}</div><div class="stat-v">${L(fmtInt(s.thread_count))}</div></div>
    <div class="panel tile"><div class="stat-k">${esc(t('rs.zombies'))}</div><div class="stat-v ${s.zombies ? 'state-z' : ''}">${L(fmtInt(s.zombies))}</div></div>
    <div class="panel tile"><div class="stat-k">${esc(t('rs.uptime'))}</div><div class="stat-v">${esc(fmtDur(s.uptime))}</div></div>`;
}

const COLS = [
  ['name', 'h_name', '34%', false], ['pid', 'h_pid', '76px', true], ['agent', 'h_group', '118px', false],
  ['cpu', 'h_cpu', '112px', true], ['memory', 'h_mem', '92px', true], ['threads', 'h_threads', '74px', true],
  ['status', 'h_state', '78px', false], ['run_secs', 'h_age', '92px', true], ['', '', '72px', false],
];

function procRow(p) {
  const cmd = p.cmd || '';
  return `<tr><td><span class="p-name">${L(p.name)}</span><span class="p-cmd" title="${esc(cmd.slice(0, 600))}">${esc(cmd.length > 180 ? cmd.slice(0, 180) + '…' : cmd)}</span></td>
    <td class="r">${L(String(p.pid))}</td><td>${p.agent ? ownerChip(p.agent) : '<span class="faint">-</span>'}</td>
    <td class="r"><span class="cpu-cell"><i style="inline-size:${Math.min(40, p.cpu / 5).toFixed(1)}px"></i>${L(fmtNum(p.cpu, 1))}</span></td>
    <td class="r">${B(p.memory)}</td><td class="r">${L(fmtInt(p.threads))}</td>
    <td class="${p.status === 'Zombie' ? 'state-z' : 'faint'}">${L(p.status)}</td><td class="r faint">${esc(fmtDur(p.run_secs))}</td>
    <td class="r"><button class="btn sm danger kill" data-act="rs-kill" data-pid="${p.pid}" data-name="${esc(p.name)}">${icon('power')}${esc(t('rs.kill'))}</button></td></tr>`;
}

function tableBody(s) {
  const q = R.q.trim().toLowerCase();
  let ps = s.processes.filter((p) => !q || p.name.toLowerCase().includes(q) || (p.cmd || '').toLowerCase().includes(q) || (p.cwd || '').toLowerCase().includes(q) || (p.agent || '').includes(q));
  const k = R.sort;
  ps.sort((a, b) => {
    const x = a[k], y = b[k];
    if (typeof x === 'string' || typeof y === 'string') return R.dir * String(x || '').localeCompare(String(y || ''));
    return R.dir * ((x || 0) - (y || 0));
  });
  if (!ps.length) return `<tr><td colspan="9" class="faint" style="padding:24px 10px">${esc(t('rs.empty'))}</td></tr>`;
  if (!R.group) return ps.slice(0, R.limit).map(procRow).join('') + more(ps.length);
  const groups = new Map();
  for (const p of ps) { const g = p.agent || ''; if (!groups.has(g)) groups.set(g, []); groups.get(g).push(p); }
  const order = [...groups.entries()].sort((a, b) => (a[0] === '') - (b[0] === '') || b[1].reduce((s, p) => s + p.cpu, 0) - a[1].reduce((s, p) => s + p.cpu, 0));
  let n = 0, out = '';
  for (const [g, list] of order) {
    const cpu = list.reduce((s, p) => s + p.cpu, 0), mem = list.reduce((s, p) => s + p.memory, 0);
    out += `<tr class="grp"><td colspan="3">${g ? ownerChip(g) : esc(t('rs.ungrouped'))}<span class="faint">${esc(t('ag.procs', { count: list.length }))}</span></td><td class="r">${L(fmtNum(cpu, 1))}</td><td class="r">${B(mem)}</td><td colspan="4"></td></tr>`;
    for (const p of list) { if (n++ >= R.limit) break; out += procRow(p); }
    if (n >= R.limit) break;
  }
  return out + more(ps.length);
}
function more(total) {
  return total > R.limit ? `<tr><td colspan="9" style="padding:10px"><button class="btn sm ghost" data-act="rs-more">${esc(t('common.show_more', { count: Math.min(100, total - R.limit) }))}</button></td></tr>` : '';
}

function headHtml() {
  return `<tr>${COLS.map(([k, label, , num]) => {
    if (!k) return '<th></th>';
    const sorted = R.sort === k;
    return `<th class="sortable ${num ? 'r' : ''}" data-act="rs-sort" data-k="${k}" ${sorted ? `aria-sort="${R.dir < 0 ? 'descending' : 'ascending'}"` : ''} tabindex="0" data-key-act="rs-sort">${esc(t('rs.' + label))}${sorted ? icon(R.dir < 0 ? 'chevronDown' : 'up') : ''}</th>`;
  }).join('')}</tr>`;
}

function paint(Lv) {
  const s = Lv.snap; if (!s || S.view !== 'resources') return;
  const top = document.getElementById('rs-top');
  if (!top) return;
  top.innerHTML = topHtml(s, Lv.hist);
  document.getElementById('rs-row2').innerHTML = row2Html(s);
  document.getElementById('rs-body').innerHTML = tableBody(s);
  const sub = document.querySelector('.topbar .tb-sub');
  if (sub) sub.innerHTML = esc(t('rs.sub', { cpu: s.cpu_brand, cores: fmtInt(s.cpu_cores.length) }));
}

const view = {
  title: () => t('rs.title'),
  sub: () => { const s = live().snap; return s ? esc(t('rs.sub', { cpu: s.cpu_brand, cores: fmtInt(s.cpu_cores.length) })) : ''; },
  noScanBtn: true,
  render(el) {
    const s = live().snap;
    el.innerHTML = `<div class="rs-top" id="rs-top">${s ? topHtml(s, live().hist) : '<div class="panel rs-cpu"><div class="skel" style="block-size:120px;grid-column:1/-1"></div></div><div class="panel gauge"><div class="skel" style="block-size:140px;inline-size:100%"></div></div><div class="panel gauge"><div class="skel" style="block-size:140px;inline-size:100%"></div></div>'}</div>
      <div class="rs-row2" id="rs-row2">${s ? row2Html(s) : ''}</div>
      ${live().err ? `<div class="err" style="margin-block-start:12px">${icon('critical')}<div>${esc(live().err)}</div></div>` : ''}
      <div class="proc-tools"><h2>${esc(t('rs.procs'))}</h2>
        <label class="search">${icon('search')}<span class="sr">${esc(t('rs.filter'))}</span><input class="input" type="search" value="${esc(R.q)}" placeholder="${esc(t('rs.filter'))}" data-input="rs-q"></label>
        <span class="spacer"></span>
        <label class="check-line"><input type="checkbox" class="cb" data-change="rs-group" ${R.group ? 'checked' : ''}>${esc(t('rs.group'))}</label>
      </div>
      <table class="ptable"><colgroup>${COLS.map((c) => `<col style="inline-size:${c[2]}">`).join('')}</colgroup><thead id="rs-head">${headHtml()}</thead><tbody id="rs-body">${s ? tableBody(s) : ''}</tbody></table>`;
  },
  mount() { R.unsub = subscribe(paint); },
  unmount() { if (R.unsub) R.unsub(); R.unsub = null; },
  acts: {
    'rs-q': (el) => { R.q = el.value; const s = live().snap; if (s) document.getElementById('rs-body').innerHTML = tableBody(s); },
    'rs-group': (el) => { R.group = el.checked; const s = live().snap; if (s) document.getElementById('rs-body').innerHTML = tableBody(s); },
    'rs-sort': (el) => {
      const k = el.dataset.k;
      if (R.sort === k) R.dir = -R.dir; else { R.sort = k; R.dir = ['name', 'agent', 'status'].includes(k) ? 1 : -1; }
      document.getElementById('rs-head').innerHTML = headHtml();
      const s = live().snap; if (s) document.getElementById('rs-body').innerHTML = tableBody(s);
    },
    'rs-more': () => { R.limit += 100; const s = live().snap; if (s) document.getElementById('rs-body').innerHTML = tableBody(s); },
    'rs-kill': (el) => confirmKill(Number(el.dataset.pid), el.dataset.name),
    'rs-kill-go': async (el) => {
      const pid = Number(el.dataset.pid);
      const force = !!document.getElementById('kill-force')?.checked;
      el.disabled = true;
      try { await api.killProcess(pid, force); toast(t('rs.killed', { pid })); closeSheet(); refreshNow(); }
      catch (e) { toast(String(e?.message || e), true); el.disabled = false; }
    },
  },
};

export function confirmKill(pid, name) {
  openSheet(() => ({
    title: t('rs.kill_t', { name }),
    body: `<p class="muted">${esc(t('rs.kill_b', { pid }))}</p>
      <label class="check-line"><input type="checkbox" class="cb" id="kill-force">${esc(t('rs.force'))}</label>`,
    foot: `<button class="btn" data-act="close-sheet">${esc(t('common.cancel'))}</button><button class="btn danger solid" data-act="rs-kill-go" data-pid="${pid}">${icon('power')}${esc(t('rs.kill_go'))}</button>`,
  }), { center: true });
}

void tilde; void home;
export default view;
