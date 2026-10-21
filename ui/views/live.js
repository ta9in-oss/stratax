// Shared resource poller: runs only while a view subscribes and the window is visible.
import { api } from '../api.js';

export const HIST = 60; // samples kept client-side (1.5 s apart = 90 s)
const L = { snap: null, hist: [], memHist: [], err: null, timer: null, subs: new Set(), busy: false };

async function tick() {
  if (document.hidden || L.busy) return;
  L.busy = true;
  try {
    const s = await api.getResources();
    L.snap = s; L.err = null;
    L.hist.push(s.cpu_total); if (L.hist.length > HIST) L.hist.shift();
    L.memHist.push(s.mem_used / s.mem_total); if (L.memHist.length > HIST) L.memHist.shift();
  } catch (e) {
    L.err = String(e?.message || e);
  } finally { L.busy = false; }
  L.subs.forEach((fn) => fn(L));
}

export function subscribe(fn) {
  L.subs.add(fn);
  if (!L.timer) { tick(); L.timer = setInterval(tick, 1500); }
  if (L.snap) fn(L);
  return () => {
    L.subs.delete(fn);
    if (!L.subs.size) { clearInterval(L.timer); L.timer = null; }
  };
}
export function live() { return L; }
export function refreshNow() { return tick(); }
