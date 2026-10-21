// Cleanup flow: always a dry run first, shown in a sheet; the real run only after confirmation.
import { S, openSheet, startScan, toast, selectedFindings } from '../app.js';
import { api } from '../api.js';
import { t } from '../i18n.js';
import { icon, esc, B, codeBlock } from '../ui.js';

const LIST_CAP = 60;

export function openCleanup(ids, mode = 'reclaim') {
  if (!ids.length) return;
  const st = { phase: 'dry', outs: [], error: null, showAll: false };
  const move = mode === 'relocate';

  const sheet = openSheet((ctl) => {
    const title = t(move ? 'sheet.title_move' : 'sheet.title');
    if (st.phase === 'dry' || st.phase === 'running') {
      const rows = Math.min(ids.length, 6);
      return {
        title,
        sub: esc(st.phase === 'dry' ? t('sheet.running_dry') : t('sheet.working')),
        body: `<div class="skel" style="block-size:40px;inline-size:220px"></div>
          <div class="outs">${Array.from({ length: rows }, () => '<div class="out"><span class="skel" style="block-size:14px"></span><span class="skel" style="block-size:14px"></span><span class="skel" style="block-size:14px;inline-size:50px"></span></div>').join('')}</div>`,
        foot: `<button class="btn" data-act="close-sheet">${esc(t('common.cancel'))}</button>`,
      };
    }
    if (st.error) {
      return {
        title,
        body: `<div class="err">${icon('critical')}<div><b>${esc(t('common.error'))}</b><div class="muted">${esc(st.error)}</div></div></div>`,
        foot: `<button class="btn" data-act="close-sheet">${esc(t('common.close'))}</button><button class="btn primary" data-act="cleanup-retry">${esc(t('common.retry'))}</button>`,
      };
    }
    const ok = st.outs.filter((o) => o.ok && !o.manual).sort((a, b) => b.freed - a.freed);
    const manual = st.outs.filter((o) => o.manual);
    const skipped = st.outs.filter((o) => !o.ok && !o.manual);
    const total = ok.reduce((s, o) => s + o.freed, 0);
    const done = st.phase === 'done';
    const shown = st.showAll ? ok : ok.slice(0, LIST_CAP);

    const outRow = (o, cls) => `<div class="out ${cls}">${icon(cls === 'ok' ? 'check' : 'minus')}<span class="o-t"><bdi>${esc(o.title)}</bdi></span><span class="o-b">${o.freed ? B(o.freed) : ''}</span><span class="o-m" title="${esc(o.message)}">${esc(o.message)}</span></div>`;

    const body = `
      ${done ? '' : `<p class="muted">${esc(t('sheet.dry'))}</p>`}
      <div class="sum"><span class="big">${B(total)}</span><span class="muted">${esc(done ? t('sheet.freed', { size: '' }).trim() : t('sheet.will_free', { size: '' }).trim())}</span></div>
      ${manual.length ? `<div class="manual-box"><h3>${icon('terminal')}${esc(t('sheet.manual_h'))}</h3>
        <div class="muted">${esc(t('sheet.manual', { count: manual.length }))}</div>
        ${codeBlock(manual.map((o) => o.manual))}</div>` : ''}
      ${ok.length ? `<div class="outs">${shown.map((o) => outRow(o, 'ok')).join('')}</div>` : ''}
      ${ok.length > shown.length ? `<button class="btn sm ghost" data-act="cleanup-all">${esc(t('common.show_more', { count: ok.length - shown.length }))}</button>` : ''}
      ${skipped.length ? `<div><div class="faint" style="margin-block-end:6px">${esc(t('sheet.blocked', { count: skipped.length }))}</div><div class="outs">${skipped.map((o) => outRow(o, 'no')).join('')}</div></div>` : ''}
      ${done ? `<p class="muted">${esc(t('sheet.done_b'))}</p>` : ''}`;
    const foot = done
      ? `<button class="btn primary" data-act="close-sheet">${esc(t('common.close'))}</button>`
      : `<span class="sf-note">${esc(t('fd.selected', { count: ids.length }))}</span><button class="btn" data-act="close-sheet">${esc(t('common.cancel'))}</button>
         <button class="btn primary" data-act="cleanup-go" ${ok.length ? '' : 'disabled'}>${icon(move ? 'move' : 'trash')}${esc(t(move ? 'sheet.go_move' : 'sheet.go', { size: '' }).trim())} ${B(total)}</button>`;
    return { title: done ? t('sheet.done_t') : title, body, foot };
  }, { onClose: () => { if (st.phase === 'done') startScan(); } });

  async function run(dry) {
    st.phase = dry ? 'dry' : 'running'; st.error = null; sheet.update();
    try {
      const runIds = dry ? ids : st.outs.filter((o) => o.ok && !o.manual).map((o) => o.id);
      const outs = await api.clean(runIds, dry, mode);
      if (dry) { st.outs = outs; st.phase = 'review'; }
      else {
        const manual = st.outs.filter((o) => o.manual);
        st.outs = outs.concat(manual); st.phase = 'done';
        const freed = outs.reduce((s, o) => s + (o.ok ? o.freed : 0), 0);
        for (const o of outs) if (o.ok) S.selected.delete(o.id);
        toast(t('sheet.freed', { size: '' }).trim() + ' ' + fmt(freed));
      }
    } catch (e) {
      st.error = String(e?.message || e); st.phase = 'review';
    }
    if (document.body.contains(sheet.el)) sheet.update();
  }
  current = { run, st, sheet };
  run(true);
}

let current = null;
import { fmtBytes as fmt } from '../i18n.js';

export const cleanupActs = {
  'cleanup-go': () => current && current.run(false),
  'cleanup-retry': () => current && current.run(true),
  'cleanup-all': () => { if (current) { current.st.showAll = true; current.sheet.update(); } },
  'clean-selected': (el) => openCleanup(selectedFindings().filter((f) => !f.protected).map((f) => f.id), el.dataset.mode || 'reclaim'),
};
