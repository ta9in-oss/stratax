import { S, setTheme, getTheme, setRoots, startScan } from '../app.js';
import { t, setLang, getLang } from '../i18n.js';
import { icon, esc } from '../ui.js';

const view = {
  title: () => t('st.title'),
  noScanBtn: true,
  render(el) {
    el.innerHTML = `<div class="st-grid">
      <div class="panel st-row"><div><h3>${esc(t('st.lang'))}</h3></div>
        <div><div class="seg" role="group" aria-label="${esc(t('st.lang'))}"><button lang="en" aria-pressed="${getLang() === 'en'}" data-act="st-lang" data-l="en">English</button><button lang="ar" aria-pressed="${getLang() === 'ar'}" data-act="st-lang" data-l="ar">العربية</button></div></div></div>
      <div class="panel st-row"><div><h3>${esc(t('st.theme'))}</h3></div>
        <div><div class="seg" role="group" aria-label="${esc(t('st.theme'))}"><button aria-pressed="${getTheme() === 'dark'}" data-act="st-theme" data-th="dark">${esc(t('st.dark'))}</button><button aria-pressed="${getTheme() === 'light'}" data-act="st-theme" data-th="light">${esc(t('st.light'))}</button></div></div></div>
      <div class="panel st-row"><div><h3>${esc(t('st.roots'))}</h3><p>${esc(t('st.roots_hint'))}</p></div>
        <div class="roots">
          ${S.roots.map((r, i) => `<div class="root-item"><div class="code"><span>${esc(r)}</span></div><button class="btn sm ghost" data-act="st-rm" data-i="${i}" aria-label="${esc(t('st.remove'))}">${icon('close')}</button></div>`).join('')}
          <form class="root-add" data-act="noop" onsubmit="return false"><input class="input" id="st-root" placeholder="${esc(t('st.root_ph'))}" spellcheck="false" aria-label="${esc(t('st.roots'))}"><button class="btn" type="button" data-act="st-add">${esc(t('st.add'))}</button></form>
          <div><button class="btn sm" data-act="rescan">${icon('refresh')}${esc(t('common.rescan'))}</button></div>
        </div></div>
      <div class="panel st-row"><div><h3>${esc(t('st.about'))}</h3></div><div><p class="muted" style="font-size:12.5px">${esc(t('st.about_b'))}</p></div></div>
    </div>`;
    const inp = el.querySelector('#st-root');
    inp && inp.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); view.acts['st-add'](); } });
  },
  acts: {
    'st-lang': (el) => setLang(el.dataset.l),
    'st-theme': (el) => setTheme(el.dataset.th),
    'st-add': () => {
      const inp = document.getElementById('st-root');
      const v = (inp?.value || '').trim();
      if (!v || S.roots.includes(v)) return;
      setRoots([...S.roots, v]); view.render(document.getElementById('view'));
    },
    'st-rm': (el) => { const i = Number(el.dataset.i); setRoots(S.roots.filter((_, j) => j !== i)); view.render(document.getElementById('view')); },
  },
};
void startScan;
export default view;
