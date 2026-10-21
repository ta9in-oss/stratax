// Stratax icon set. Drawn on a 24px grid, 1.6 stroke, round caps and joins.
// Icons marked in DIRECTIONAL are mirrored in RTL via the .ico-dir class.

const P = {
  // navigation
  overview: '<path d="M4 6.5h16M4 12h11M4 17.5h7"/><circle cx="18.5" cy="16" r="2.5"/>',
  findings: '<rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="M7.5 9h2M7.5 12.5h2M7.5 16h2M12 9h4.5M12 12.5h4.5M12 16h2.5"/>',
  agents: '<rect x="5" y="7" width="14" height="11" rx="3"/><path d="M12 7V4M9.5 12v1.5M14.5 12v1.5M2.5 12v2.5M21.5 12v2.5"/><circle cx="12" cy="3.5" r=".6" fill="currentColor"/>',
  resources: '<path d="M3 12h3.5l2.5-6 4 12 2.5-6H21"/>',
  explorer: '<rect x="3.5" y="3.5" width="17" height="17" rx="2"/><path d="M12.5 3.5v17M12.5 11h8M3.5 14h9M16.5 11v9.5"/>',
  advisor: '<path d="M5 18.5V7a2.5 2.5 0 0 1 2.5-2.5h9A2.5 2.5 0 0 1 19 7v6.5a2.5 2.5 0 0 1-2.5 2.5H9l-4 2.5Z"/><path d="M9 9.5h6M9 12.5h3.5"/>',
  settings: '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>',
  // ui
  chevron: '<path d="m9.5 6 6 6-6 6"/>',
  chevronDown: '<path d="m6 9.5 6 6 6-6"/>',
  arrow: '<path d="M5 12h14M13.5 6.5 19 12l-5.5 5.5"/>',
  search: '<circle cx="11" cy="11" r="6"/><path d="m15.5 15.5 4 4"/>',
  close: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
  check: '<path d="m5.5 12.5 4 4 9-9"/>',
  minus: '<path d="M6.5 12h11"/>',
  lock: '<rect x="5.5" y="10.5" width="13" height="9" rx="2"/><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5"/>',
  copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2"/><path d="M15.5 8.5V6.5a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/>',
  refresh: '<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v4h-4"/>',
  stop: '<rect x="7" y="7" width="10" height="10" rx="1.5"/>',
  sun: '<circle cx="12" cy="12" r="3.5"/><path d="M12 3.5v1.5M12 19v1.5M3.5 12H5M19 12h1.5M6 6l1 1M17 17l1 1M6 18l1-1M17 7l1-1"/>',
  moon: '<path d="M19 14.5A7.5 7.5 0 0 1 9.5 5a7.5 7.5 0 1 0 9.5 9.5Z"/>',
  lang: '<path d="M4 6h9M8.5 4v2M11 6c-.6 3.6-3 6.6-6 8M7 9c1 2 2.8 3.7 5 4.6"/><path d="m13 20 3.5-8 3.5 8M14.3 17h4.4"/>',
  filter: '<path d="M4 6h16M7 12h10M10 18h4"/>',
  sort: '<path d="M8 5v14M5 16l3 3 3-3M16 19V5M13 8l3-3 3 3"/>',
  sidebar: '<rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="M9.5 4.5v15"/>',
  terminal: '<rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="m7.5 9.5 3 2.5-3 2.5M12.5 15h4"/>',
  folder: '<path d="M3.5 7.5a2 2 0 0 1 2-2h3.8l2 2h7.2a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2Z"/>',
  file: '<path d="M7 3.5h6.5l4 4V19a1.5 1.5 0 0 1-1.5 1.5H7A1.5 1.5 0 0 1 5.5 19V5A1.5 1.5 0 0 1 7 3.5Z"/><path d="M13.5 3.5v4h4"/>',
  trash: '<path d="M4.5 7h15M10 7V5h4v2M6.5 7l1 12.5h9l1-12.5"/>',
  move: '<path d="M4 12h11M11 7.5l4.5 4.5-4.5 4.5"/><path d="M19.5 5v14"/>',
  power: '<path d="M12 4v7"/><path d="M7.5 7a7 7 0 1 0 9 0"/>',
  up: '<path d="M12 19V5M6.5 10.5 12 5l5.5 5.5"/>',
  home: '<path d="M4.5 10.5 12 4.5l7.5 6V19a1 1 0 0 1-1 1h-4v-5.5h-5V20h-4a1 1 0 0 1-1-1Z"/>',
  // safety
  safe: '<path d="M12 3.5 19 6v5.5c0 4.2-3 7.5-7 9-4-1.5-7-4.8-7-9V6Z"/><path d="m9 12 2 2 4-4"/>',
  rebuild: '<path d="M19 12a7 7 0 0 1-12.2 4.7M5 12a7 7 0 0 1 12.2-4.7"/><path d="M17.5 3.5v4h-4M6.5 20.5v-4h4"/>',
  review: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z"/><circle cx="12" cy="12" r="2.8"/>',
  // severity
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.5M12 7.8v.4"/>',
  warn: '<path d="M10.3 4.6 3 17.5A2 2 0 0 0 4.7 20.5h14.6a2 2 0 0 0 1.7-3L13.7 4.6a2 2 0 0 0-3.4 0Z"/><path d="M12 9.5v4.5M12 17v.3"/>',
  critical: '<path d="M8.3 3.5h7.4l4.8 4.8v7.4l-4.8 4.8H8.3l-4.8-4.8V8.3Z"/><path d="M12 8v5M12 16v.3"/>',
  // hardware
  hdd: '<rect x="3.5" y="5.5" width="17" height="13" rx="2"/><circle cx="12" cy="11.5" r="3.5"/><path d="M12 11.5h.01M6.5 16h3"/>',
  ssd: '<rect x="3.5" y="7" width="17" height="10" rx="1.5"/><path d="M7 10.5h3v3H7zM13 10.5h4M13 13.5h4"/>',
  unplug: '<path d="M9 4v4M15 4v4M7 8h10v3a5 5 0 0 1-10 0Z"/><path d="M12 16v4"/><path d="M4 4l16 16"/>',
  cpu: '<rect x="6.5" y="6.5" width="11" height="11" rx="1.5"/><path d="M9.5 9.5h5v5h-5zM9.5 3v3.5M14.5 3v3.5M9.5 17.5V21M14.5 17.5V21M3 9.5h3.5M3 14.5h3.5M17.5 9.5H21M17.5 14.5H21"/>',
  memory: '<rect x="3.5" y="7" width="17" height="9" rx="1.5"/><path d="M7 16v2.5M11 16v2.5M15 16v2.5M7 10v3M10.5 10v3M14 10v3M17.5 10v3"/>',
  mark: '<path d="M4 7.5h11"/><path d="M7 12h13"/><path d="M4 16.5h8"/>',
};

const DIRECTIONAL = new Set(['chevron', 'arrow', 'move']);

export function icon(name, cls = '') {
  const body = P[name] || P.info;
  const dir = DIRECTIONAL.has(name) ? ' ico-dir' : '';
  return `<svg class="ico${dir} ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${body}</svg>`;
}
