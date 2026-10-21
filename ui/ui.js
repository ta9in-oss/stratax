// Small rendering helpers shared by all views.
import { t, fmtBytes, fmtSpan } from './i18n.js';
import { icon } from './icons/icons.js';

export { icon };

export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

/** Size in an LTR isolate so units never flip in RTL. */
export const B = (b, cls = '') => `<bdi class="num ${cls}" dir="ltr">${esc(fmtBytes(b))}</bdi>`;
export const L = (s, cls = '') => `<bdi dir="ltr" class="${cls}">${esc(s)}</bdi>`;

let HOME = null;
export function setHome(h) { HOME = h; }
export function home() { return HOME; }
export function tilde(p) {
  if (!p) return '';
  if (HOME && (p === HOME || p.startsWith(HOME + '/'))) return '~' + p.slice(HOME.length);
  return p;
}
export function midTrunc(s, n = 64) {
  if (s.length <= n) return s;
  const head = Math.ceil((n - 1) * 0.45), tail = n - 1 - head;
  return s.slice(0, head) + '…' + s.slice(s.length - tail);
}

const SAFETY_ICON = { safe: 'safe', rebuild: 'rebuild', review: 'review' };
export function safetyBadge(s) {
  return `<span class="badge ${s}" title="${esc(t('safety.' + s + '_d'))}">${icon(SAFETY_ICON[s] || 'info')}${esc(t('safety.' + s))}</span>`;
}
export function verdictBadge(v) {
  const ic = { reclaim: 'trash', relocate: 'move', keep: 'lock', review: 'review' }[v] || 'info';
  return `<span class="badge ${v}">${icon(ic)}${esc(t('verdict.' + v))}</span>`;
}

export const AI_OWNERS = new Set(['claude-code', 'claude-desktop', 'codex', 'cursor', 'vscode', 'cargo-mutants', 'copilot', 'windsurf', 'aider', 'gemini']);
export function ownerChip(o) {
  const name = o || t('owner.none');
  return `<span class="owner ${AI_OWNERS.has(o) ? 'ai' : ''}">${L(name)}</span>`;
}

export const CATS = ['build_artifact', 'package_cache', 'container', 'browser_data', 'app_cache', 'agent_data', 'toolchain', 'system', 'models', 'worktree', 'mutation_test'];
export const catColor = (c) => `var(--c-${CATS.includes(c) ? c : 'other'})`;
export const catLabel = (c) => t('cat.' + c);

/** Inline text with `code` spans rendered mono and isolated. */
export function rich(s) {
  return esc(s).replace(/`([^`]+)`/g, '<code class="inl">$1</code>');
}

export function codeLine(cmd, prompt = '$') {
  return `<div class="code"><span class="prompt">${esc(prompt)}</span><span>${esc(cmd)}</span><button class="btn sm ghost" data-act="copy" data-copy="${esc(cmd)}" aria-label="${esc(t('common.copy'))}">${icon('copy')}<span>${esc(t('common.copy'))}</span></button></div>`;
}

export function idleText(unix, now) {
  if (!unix) return t('common.never');
  return t('fd.idle', { age: fmtSpan(Math.max(60, now - unix)) });
}

export function isFsPath(p) { return typeof p === 'string' && p.startsWith('/'); }
export function toolOf(p) { const m = /^(\w+):\/\//.exec(p || ''); return m ? m[1] : null; }

/** Multi-line shell script with one copy button for the whole thing. */
export function codeBlock(lines) {
  const txt = lines.join('\n');
  return `<div class="code multi"><pre>${lines.map((l) => `<span class="prompt">$</span> ${esc(l)}`).join('\n')}</pre><button class="btn sm ghost" data-act="copy" data-copy="${esc(txt)}">${icon('copy')}<span>${esc(t('common.copy'))}</span></button></div>`;
}

/** Bytes a selection frees, counting a finding nested in another selected one once
 *  (e.g. a Rust target/ and its own incremental/ cache). Mirrors clean::selection_bytes. */
export function selectionBytes(list) {
  const fs = list.filter((f) => !String(f.path).startsWith('docker://'));
  return list
    .filter((f) => !fs.some((o) => o.id !== f.id && f.path !== o.path && String(f.path).startsWith(o.path + '/')))
    .reduce((s, f) => s + f.bytes, 0);
}
