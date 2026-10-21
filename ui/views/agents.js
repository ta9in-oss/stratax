import { S, go, touchSelection } from '../app.js';
import { t, fmtPct, fmtInt, fmtAge } from '../i18n.js';
import { icon, esc, B, L, ownerChip, catColor, tilde, codeLine, CATS } from '../ui.js';
import { subscribe, live } from './live.js';
import { revealFindings } from './findings.js';

const AI = ['claude-code', 'claude-desktop', 'codex', 'cursor', 'vscode', 'copilot', 'windsurf', 'aider', 'gemini'];
const WORK = ['rust', 'node', 'python', 'go', 'docker', 'cargo-mutants', 'flutter', 'php', 'c++', 'android', 'terraform', 'ml', 'java', 'dotnet'];
const A = { unsub: null };

function bucket(o) { if (AI.includes(o)) return 'ai'; if (WORK.includes(o)) return 'work'; return 'sys'; }

function footprint(r) {
  const m = new Map();
  for (const f of r.findings) {
    const k = f.owner || '';
    if (!m.has(k)) m.set(k, { owner: f.owner, bytes: 0, cats: {} });
    const e = m.get(k); e.bytes += f.bytes; e.cats[f.category] = (e.cats[f.category] || 0) + f.bytes;
  }
  const all = [...m.values()].sort((a, b) => b.bytes - a.bytes);
  const max = all.length ? all[0].bytes : 1;
  const sec = { ai: [], work: [], sys: [] };
  all.forEach((e) => sec[bucket(e.owner)].push(e));
  const line = (e) => `<div class="fp" title="${esc(Object.entries(e.cats).map(([c, b]) => t('cat.' + c)).join(', '))}">${ownerChip(e.owner)}
    <div class="fp-bar" style="inline-size:${Math.max(0.5, (e.bytes / max) * 100).toFixed(2)}%">${CATS.filter((c) => e.cats[c]).map((c) => `<i style="flex:${e.cats[c]};background:${catColor(c)}"></i>`).join('')}</div>${B(e.bytes)}</div>`;
  const block = (k, list, cap) => (list.length ? `<div class="fp-group"><h3>${esc(t('ag.' + k))} · ${B(list.reduce((s, e) => s + e.bytes, 0))}</h3>${list.slice(0, cap).map(line).join('')}</div>` : '');
  const usedCats = CATS.filter((c) => r.findings.some((f) => f.category === c));
  return `<div class="panel"><div class="panel-h"><h2>${esc(t('ag.footprint'))}</h2></div>
    <div class="panel-b" style="padding-block-start:4px"><p class="faint" style="font-size:12px;margin-block-end:14px">${esc(t('ag.footprint_sub'))}</p>
    ${block('ai', sec.ai, 8)}${block('work', sec.work, 9)}${block('sys', sec.sys, 6)}
    <div class="cap-legend" style="margin-block-start:16px">${usedCats.map((c) => `<span><i class="sw" style="background:${catColor(c)}"></i>${esc(t('cat.' + c))}</span>`).join('')}</div></div></div>`;
}

function cleanCwds(list) {
  return (list || []).filter((c) => c && !c.startsWith('/proc') && !c.startsWith('/usr') && !/\/registry\/src\//.test(c));
}

function fp(a) {
  if (a.footprint) return a.footprint;
  const r = S.report && S.report.agents.find((x) => x.agent === a.agent);
  return r ? r.footprint : 0;
}

function runningHtml(agents) {
  const list = [...agents].sort((a, b) => (a.kind === 'agent' ? 0 : a.kind === 'workload' ? 1 : 2) - (b.kind === 'agent' ? 0 : b.kind === 'workload' ? 1 : 2) || b.cpu - a.cpu);
  if (!list.length) return `<p class="muted">${esc(t('ag.no_agents'))}</p>`;
  return list.map((a) => {
    const cwds = cleanCwds(a.cwds);
    return `<div class="panel lcard"><div class="lcard-h"><span class="l-name">${L(a.agent)}</span><span class="badge k-${a.kind}">${esc(t('kind.' + a.kind))}</span>
      <span class="faint" style="margin-inline-start:auto;font-size:12px">${esc(t('ag.procs', { count: a.processes }))}</span></div>
      <div class="lcard-stats">
        <div><div class="stat-k">${esc(t('ag.cpu'))}</div><div class="stat-v">${L(fmtPct(a.cpu / 100))}</div></div>
        <div><div class="stat-k">${esc(t('ag.mem'))}</div><div class="stat-v">${B(a.memory)}</div></div>
        <div><div class="stat-k">${esc(t('ag.threads'))}</div><div class="stat-v">${L(fmtInt(a.threads))}</div></div>
        <div><div class="stat-k">${esc(t('ag.on_disk'))}</div><div class="stat-v">${fp(a) ? B(fp(a)) : '<span class="faint">-</span>'}</div></div>
      </div>
      ${cwds.length ? `<div class="cwds">${cwds.slice(0, 6).map((c) => `<span class="cwd" title="${esc(c)}">${esc(tilde(c))}</span>`).join('')}</div>` : ''}
    </div>`;
  }).join('');
}

function projectsHtml(r, agents) {
  const m = new Map();
  for (const a of agents) {
    if (a.kind === 'infra') continue;
    for (const c of cleanCwds(a.cwds)) {
      if (/^\/home\/[^/]+$/.test(c) || c.startsWith('/tmp/')) continue;
      if (!m.has(c)) m.set(c, new Set());
      m.get(c).add(a.agent);
    }
  }
  const rows = [...m.entries()].map(([p, who]) => {
    const fs = r.findings.filter((f) => f.path.startsWith(p + '/'));
    return { p, who: [...who], bytes: fs.reduce((s, f) => s + f.bytes, 0), ids: fs.map((f) => f.id) };
  }).sort((a, b) => b.bytes - a.bytes);
  if (!rows.length) return '';
  return `<div class="panel"><div class="panel-h"><h2>${esc(t('ag.projects'))}</h2></div><div class="panel-b" style="padding-block-start:4px">
    <p class="faint" style="font-size:12px;margin-block-end:6px">${esc(t('ag.projects_sub'))}</p>
    ${rows.map((x) => `<div class="proj"><span class="p-path" title="${esc(x.p)}">${esc(tilde(x.p))}</span>
      ${x.bytes ? `<button class="btn sm ghost num" data-act="ag-reveal" data-ids="${x.ids.join(',')}">${B(x.bytes)}</button>` : `<span class="faint" style="grid-row:span 2;align-self:center;font-size:12px">${esc(t('ag.no_output'))}</span>`}
      <span class="p-who">${x.who.map(ownerChip).join('')}</span></div>`).join('')}
  </div></div>`;
}

function mutationHtml(r) {
  const left = r.findings.filter((f) => f.category === 'mutation_test' || f.category === 'worktree');
  return `<div class="panel" style="margin-block-start:16px"><div class="panel-h"><h2>${esc(t('ag.mut_t'))}</h2></div>
    <div class="panel-b mut">
      <div style="display:grid;gap:12px">
        <ol class="mut-steps"><li>${esc(t('ag.mut_b1'))}</li><li>${esc(t('ag.mut_b2'))}</li></ol>
        <p>${esc(t('ag.mut_fix'))}</p>
        ${codeLine('CARGO_TARGET_DIR=/tmp/mutants-target CARGO_INCREMENTAL=0 cargo mutants')}
        ${codeLine('rm -rf /tmp/mutants-target /tmp/cargo-mutants-*.tmp')}
      </div>
      <div><h3 style="font-size:12px;color:var(--text-3);font-weight:500;margin-block-end:6px">${esc(t('ag.leftovers'))}</h3>
        ${left.length ? left.map((f) => `<div class="proj"><span class="p-path" title="${esc(f.path)}">${esc(tilde(f.path))}</span>
          <button class="btn sm ghost num" data-act="ag-reveal" data-ids="${f.id}">${B(f.bytes)}</button>
          <span class="p-who">${ownerChip(f.owner)}<span class="faint" style="font-size:11.5px">${esc(fmtAge(f.last_activity))}</span></span></div>`).join('') : `<p class="muted">${esc(t('ag.no_leftovers'))}</p>`}
      </div>
    </div></div>`;
}

function paintLive(Lv) {
  if (S.view !== 'agents' || !Lv.snap) return;
  const el = document.getElementById('ag-live');
  if (el) el.innerHTML = runningHtml(Lv.snap.agents);
}

export default {
  title: () => t('ag.title'),
  sub: () => esc(t('ag.sub')),
  render(el) {
    const r = S.report;
    if (!r) { el.innerHTML = `<div class="ag-grid"><div class="panel"><div class="skel" style="block-size:320px"></div></div><div class="panel"><div class="skel" style="block-size:320px"></div></div></div>`; return; }
    const agents = live().snap ? live().snap.agents : r.agents;
    el.innerHTML = `<div class="ag-grid">
      <div style="display:grid;gap:16px">${footprint(r)}${projectsHtml(r, r.agents)}</div>
      <div><div class="section-t" style="margin-block-start:0">${esc(t('ag.running'))}<span class="st-sub">${esc(t('ag.running_sub'))}</span></div><div class="live" id="ag-live">${runningHtml(agents)}</div></div>
    </div>${mutationHtml(r)}`;
  },
  mount() { A.unsub = subscribe(paintLive); },
  unmount() { if (A.unsub) A.unsub(); A.unsub = null; },
  acts: {
    'ag-reveal': (el) => {
      const ids = el.dataset.ids.split(',').filter(Boolean);
      revealFindings(ids); touchSelection();
      ids.forEach((id) => { const f = S.report.findings.find((x) => x.id === id); if (f && !f.protected) S.selected.add(id); });
      go('findings');
    },
  },
};
