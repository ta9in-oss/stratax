// Backend adapter. Inside Tauri it forwards to Rust commands; in a plain
// browser it serves ./mock/*.json and simulates progress so the UI is testable.

const T = window.__TAURI__;
export const isMock = !T;

const progressSubs = new Set();
/** Subscribe to scan/tree walk progress ({files, bytes}). Returns an unsubscribe fn. */
export function onProgress(fn) { progressSubs.add(fn); return () => progressSubs.delete(fn); }
const emit = (p) => progressSubs.forEach((fn) => fn(p));

if (T) {
  T.event.listen('scan-progress', (e) => emit(e.payload)).catch(() => {});
}

const invoke = (cmd, args) => T.core.invoke(cmd, args);

// ---------------------------------------------------------------- mock
const mock = {
  cache: {},
  removed: new Set(),
  killed: new Set(),
  config: { provider: 'claude_code', model: null, base_url: null, api_key_env: null },
  cancel: false,
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function load(name) {
  if (!mock.cache[name]) {
    const res = await fetch(`./mock/${name}.json`);
    if (!res.ok) throw new Error(`mock ${name}: ${res.status}`);
    mock.cache[name] = await res.json();
  }
  return structuredClone(mock.cache[name]);
}

async function mockReport() {
  const r = await load('report');
  if (mock.removed.size) {
    const gone = r.findings.filter((f) => mock.removed.has(f.id));
    r.findings = r.findings.filter((f) => !mock.removed.has(f.id));
    for (const f of gone) {
      const tot = r.totals.find((x) => x.category === f.category);
      if (tot) { tot.bytes -= f.bytes; tot.reclaimable = Math.max(0, tot.reclaimable - f.bytes); tot.count -= 1; }
    }
    const root = r.disks.find((d) => d.mount === '/');
    const freed = gone.reduce((s, f) => s + f.bytes, 0);
    if (root) { root.used -= freed; root.available += freed; }
  }
  r.generated_at = Math.floor(Date.now() / 1000) - 5;
  return r;
}

async function simulateWalk(totalFiles, totalBytes, ms) {
  mock.cancel = false;
  const t0 = performance.now();
  for (;;) {
    const k = Math.min(1, (performance.now() - t0) / ms);
    const e = 1 - Math.pow(1 - k, 2);
    emit({ files: Math.round(totalFiles * e), bytes: Math.round(totalBytes * e) });
    if (k >= 1 || mock.cancel) break;
    await sleep(120);
  }
}

function jitter(v, amt, lo = 0, hi = Infinity) { return Math.min(hi, Math.max(lo, v + (Math.random() * 2 - 1) * amt)); }

function mockTree(report, path) {
  const root = { name: path, bytes: 0, files: 0, dirs: 0, newest: 0, children: [], other_bytes: 0, top_files: [] };
  const base = path.replace(/\/$/, '');
  for (const f of report.findings) {
    if (!f.path.startsWith(base + '/')) continue;
    const parts = f.path.slice(base.length + 1).split('/');
    let node = root;
    const touch = (n) => { n.bytes += f.bytes; n.files += f.files; n.newest = Math.max(n.newest, f.last_activity || 0); };
    touch(node);
    for (const part of parts) {
      let c = node.children.find((x) => x.name === part);
      if (!c) { c = { name: part, bytes: 0, files: 0, dirs: 0, newest: 0, children: [], other_bytes: 0, top_files: [] }; node.children.push(c); node.dirs++; }
      touch(c); node = c;
    }
    // plausible contents for leaves so the treemap has depth
    if (/\/target$/.test(f.path) && !node.children.length) {
      const parts2 = [['debug', 0.62], ['release', 0.2], ['mutants.out', 0.04]];
      for (const [n, w] of parts2) node.children.push({ name: n, bytes: Math.round(f.bytes * w), files: Math.round(f.files * w), dirs: 4, newest: f.last_activity || 0, children: [], other_bytes: 0, top_files: [] });
      node.other_bytes = Math.round(f.bytes * 0.14);
    }
  }
  const sortRec = (n) => { n.children.sort((a, b) => b.bytes - a.bytes); n.children.forEach(sortRec); };
  sortRec(root);
  return root;
}

// ---------------------------------------------------------------- api
export const api = {
  async lastReport() {
    if (T) return invoke('last_report');
    return mockReport();
  },
  async runScan(roots) {
    if (T) return invoke('run_scan', { roots: roots && roots.length ? roots : null });
    const r = await mockReport();
    await simulateWalk(2_418_903, r.findings.reduce((s, f) => s + f.bytes, 0) * 1.6, 3200);
    if (mock.cancel) { r.scan_status = 'cancelled'; }
    r.duration_ms = 3200;
    return r;
  },
  async cancelScan() {
    if (T) return invoke('cancel_scan');
    mock.cancel = true;
  },
  async getTree(path) {
    if (T) return invoke('get_tree', { path });
    const r = await mockReport();
    const tree = mockTree(r, path);
    await simulateWalk(tree.files || 120000, tree.bytes || 1e9, 1400);
    return tree;
  },
  async getResources() {
    if (T) return invoke('get_resources');
    const r = await load('resources');
    r.at = Math.floor(Date.now() / 1000);
    r.processes = r.processes.filter((p) => !mock.killed.has(p.pid));
    r.cpu_cores = r.cpu_cores.map((c) => jitter(c, 14, 1, 100));
    r.cpu_total = r.cpu_cores.reduce((s, c) => s + c, 0) / r.cpu_cores.length;
    r.mem_used = jitter(r.mem_used, 180e6, 1e9, r.mem_total * 0.95);
    r.mem_available = r.mem_total - r.mem_used;
    r.swap_used = jitter(r.swap_used, 40e6, 0, r.swap_total);
    r.load = r.load.map((l) => jitter(l, 0.6, 0.1));
    if (!mock.t0) mock.t0 = Date.now();
    r.uptime += Math.floor((Date.now() - mock.t0) / 1000);
    for (const p of r.processes) { if (p.cpu > 0.5) p.cpu = jitter(p.cpu, Math.max(2, p.cpu * 0.25), 0); p.memory = jitter(p.memory, p.memory * 0.01, 0); }
    for (const a of r.agents) { a.cpu = jitter(a.cpu, Math.max(2, a.cpu * 0.2), 0); }
    return r;
  },
  async killProcess(pid, force) {
    if (T) return invoke('kill_process', { pid, force });
    await sleep(250);
    mock.killed.add(pid);
  },
  async getAdvisorConfig() {
    if (T) return invoke('get_advisor_config');
    return { ...mock.config };
  },
  async setAdvisorConfig(config) {
    if (T) return invoke('set_advisor_config', { config });
    mock.config = { ...config };
  },
  async advise(config) {
    if (T) return invoke('advise', { config: config || null });
    await sleep(config && config.provider === 'offline' ? 700 : 4200);
    const a = await load('advice');
    if (config && config.provider !== 'offline') a.provider = config.model || config.provider;
    return a;
  },
  async clean(ids, dryRun, mode) {
    if (T) return invoke('clean', { ids, dryRun, mode: mode || null });
    const r = await mockReport();
    await sleep(dryRun ? 700 : 1600);
    const out = [];
    for (const id of ids) {
      const f = r.findings.find((x) => x.id === id);
      if (!f) continue;
      const a = f.action;
      if (f.protected) { out.push({ id, title: f.title, ok: false, freed: 0, message: `protected: ${f.protected}`, manual: null }); continue; }
      if (a.kind === 'review_only') { out.push({ id, title: f.title, ok: false, freed: 0, message: 'review only, not touched', manual: null }); continue; }
      if (a.kind === 'run_tool' && a.program === 'sudo') {
        out.push({ id, title: f.title, ok: true, freed: 0, message: 'needs root', manual: ['sudo', ...a.args].join(' ') });
        continue;
      }
      const verb = a.kind === 'run_tool' ? `${dryRun ? 'would run' : 'ran'} ${a.program} ${a.args.join(' ')}` : `${dryRun ? 'would delete' : 'deleted'} ${a.paths.length} path`;
      out.push({ id, title: f.title, ok: true, freed: f.bytes, message: verb, manual: null });
      if (!dryRun) mock.removed.add(id);
    }
    return out;
  },
};
