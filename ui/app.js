'use strict';
// Another Life frontend: plain JS, no build step. The whole run lives in `S.run`;
// the Rust backend is stateless and only reads saves, validates, saves runs and exports.

const TAURI = window.__TAURI__;

// ---------------------------------------------------------------- backend glue
// Browser preview (no Tauri): `w3al questbook-json > ui/dev-questbook.json`, then serve ui/.
// The mocks below only exist so every screen can be looked at; nothing is read or written.
async function mockInvoke(cmd, args) {
  if (typeof editMock === 'function') { const r = await editMock(cmd, args); if (r !== undefined) return r; } // ui/edit.js
  if (cmd === 'load_questbook') return (await fetch('dev-questbook.json')).json();
  if (cmd === 'load_images') return (await fetch('dev-images.json')).json();
  if (cmd === 'validate_run') return { effects: [], completed: [], started: [], blocked: [], diagnostics: [] };
  if (cmd === 'default_paths') return { gamesaves: 'C:\\Users\\you\\Documents\\The Witcher 3\\gamesaves (preview)', gamesaves_exists: false, game_dir: 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\The Witcher 3' };
  if (cmd === 'donor_status') return [];
  if (cmd === 'template_plan') return { dirs: ['templates (preview)'], choice: null, error: 'the browser preview reads no saves', templates: [] };
  if (cmd === 'detect_game') return {
    path: args.savePath, file_size: 5242880, chunk_count: 42, header_ints: [67, 29, 164], looks_like_5x: true,
    payload_error: null, fact_count: 0, name_count: 0, world: 'levels\\novigrad\\novigrad.w2w (preview)',
    journal_entries: 0, journal_counters: 0, tracked_quest: '(browser preview: nothing was read)', journal_error: null, issues: [],
  };
  if (cmd === 'check_export' || cmd === 'export_save') throw {
    kind: 'setup', title: 'Export needs the desktop app.', hint: 'The browser preview only shows the screens.', detail: cmd,
  };
  throw new Error('"' + cmd + '" needs the desktop app (browser preview only shows the story pages).');
}
const invoke = TAURI ? TAURI.core.invoke : mockInvoke;

async function pickFile(filters) {
  if (TAURI) return TAURI.dialog.open({ multiple: false, directory: false, filters });
  return window.prompt('Path:') || null;
}
async function pickDir(title) {
  if (TAURI) return TAURI.dialog.open({ multiple: false, directory: true, title });
  return window.prompt(title || 'Folder:') || null;
}
async function pickSavePath(defaultPath) {
  if (TAURI) return TAURI.dialog.save({ defaultPath, filters: [{ name: 'Run profile', extensions: ['toml'] }] });
  return window.prompt('Save run as:', defaultPath) || null;
}

// ---------------------------------------------------------------- state
// Texts come from ui/i18n/<lang>.json through t() (ui/i18n.js).
const regionName = r => (t.has('region.' + r) ? t('region.' + r) : r);
// Display only: the level file names seen in real saves' `worldInfo/world`.
const worldName = s => (t.has('world.' + s) ? t('world.' + s) : null);
const sealText = s => t('seal.' + s);
// Official names read from the player's own game install (backend `game_names`); empty without one.
const qname = q => (S.names.quests && S.names.quests[q.id]) || loc(q, 'title');
const iname = n => (S.names.items && S.names.items[n]) || n;
async function loadNames() {
  try { S.names = await invoke('game_names', { lang: I18N.lang, gameDir: S.run.game_dir }); } catch (_) { S.names = { found: false, quests: {}, items: {} }; }
  try { await GameText.load(I18N.lang, S.run.game_dir); } catch (_) { /* no journal texts: our own summaries only */ }
}

// Our own translations of the questbook text (ui/i18n/questbook.<lang>.json) become `<field>_<lang>` fields, which loc() reads.
async function loadBookTranslations(book) {
  for (const lang of Object.keys(I18N.LANGS).filter(l => l !== 'en')) {
    let tr;
    try { tr = await (await fetch('i18n/questbook.' + lang + '.json')).json(); } catch (_) { continue; }
    for (const q of book.quests) {
      const x = tr[q.id];
      if (!x) continue;
      for (const f of ['title', 'summary']) if (x[f]) q[f + '_' + lang] = x[f];
      for (const c of q.choices || []) {
        const xc = x.choices && x.choices[c.id];
        if (!xc) continue;
        if (xc.prompt) c['prompt_' + lang] = xc.prompt;
        for (const o of c.options || []) if (xc.options && xc.options[o.id]) o['text_' + lang] = xc.options[o.id];
      }
    }
  }
}

function newRun() {
  // allow_name_remap defaults ON in the app (the core's default for hand-written run files is off).
  // template: null = chosen automatically from questbook/templates.toml (a picked save is a manual override).
  return { version: 1, w2_import: false, template: null, template_dirs: [], stop_after: null, quests: {}, donors: [], allow_name_remap: true, game_dir: null };
}

const S = {
  images: {},          // questbook/images.toml: quest id | act:<n> | region:<id> -> { image, page, title }
  screen: 'welcome',
  book: null,          // { quests, warnings, diagnostics }
  run: newRun(),
  game: null,          // detect_game result for S.run.template
  paths: null,         // default_paths result
  idx: 0,              // current quest index
  step: 0,             // 0 = status, 1..n = choices
  maxIdx: -1,          // furthest quest reached
  filter: 'all',
  list: null,          // active option list: { items, sel, confirm }
  exportDir: '',
  exportResult: null,  // { ok: report } | { fail: failure }
  donors: null,        // donor_status result
  validation: null,
  busy: false,
  plan: null,          // template_plan result (automatic template choice; used when run.template is null)
  names: { found: false, quests: {}, items: {} }, // game_names result: official names from the game install
};

// ---------------------------------------------------------------- tiny DOM helper
function h(tag, props, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v === false || v == null) continue;
    if (k === 'class') el.className = v;
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'checked') el.checked = !!v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat()) {
    if (kid == null || kid === false) continue;
    el.append(kid.nodeType ? kid : document.createTextNode(kid));
  }
  return el;
}
const app = document.getElementById('app');
function mount(...nodes) { app.replaceChildren(...nodes); app.scrollTop = 0; nav(); }

function banner(kind, msg) {
  const b = document.getElementById('banner');
  b.hidden = !msg;
  b.className = 'banner ' + (kind || '');
  b.textContent = msg || '';
}
function hints(...pairs) {
  document.getElementById('hints').replaceChildren(
    ...pairs.map(([k, label]) => h('span', null, h('kbd', null, k), ' ', t(label))));
}
// ---------------------------------------------------------------- chrome: sidebar, top bar, ledger
const ROMAN = ['', 'I', 'II', 'III', 'IV', 'V', 'VI', 'VII', 'VIII', 'IX', 'X', 'XI', 'XII'];
function actGroups() {
  const by = new Map();
  (S.book ? quests() : []).forEach((q, i) => {
    if (!by.has(q.act)) by.set(q.act, { act: q.act, idxs: [], regions: {} });
    const g = by.get(q.act);
    g.idxs.push(i);
    g.regions[q.region] = (g.regions[q.region] || 0) + 1;
  });
  return [...by.values()].sort((a, b) => a.act - b.act).map(g => ({
    ...g,
    label: regionName(Object.entries(g.regions).sort((a, b) => b[1] - a[1])[0][0]),
    done: g.idxs.filter(i => entryOf(quests()[i].id)).length,
  }));
}
function pendingItems() {
  if (!S.book) return [];
  const out = [];
  quests().forEach((q, i) => { const e = entryOf(q.id); if (e) out.push({ q, i, e }); });
  return out;
}
function kitCount() { return Object.keys(S.run.start_kit || {}).length; }
function pendingCount() { return pendingItems().length + (S.run.w2_import ? 1 : 0) + kitCount(); }
function answerLine(q, e) {
  if (e.status !== 'completed') return e.status === 'started' ? t('answer.started') : t('answer.skipped');
  const ch = q.choices || [];
  if (!ch.length) return t('answer.completed');
  return ch.map(c => optionText(q, c.id, (e.answers || {})[c.id] || c.default)).join('; ');
}
function sealOf(q, e) {
  if (!e) return 'notreached';
  if (e.status === 'completed' && (q.choices || []).some(c => (e.answers || {})[c.id])) return 'decided';
  return 'default';
}
// (seal labels: sealText(), texts in ui/i18n/*.json)

function refreshAfterUndo() {
  S.step = 0;
  S.maxIdx = Math.max(-1, ...pendingItems().map(x => x.i), S.screen === 'quest' || S.screen === 'checkpoint' ? S.idx : -1);
  S.exportResult = null;
  go(S.screen === 'checkpoint' ? 'quest' : S.screen);
}
function undoQuest(id) {
  delete S.run.quests[id];
  if (S.run.stop_after === id) S.run.stop_after = null;
  refreshAfterUndo();
}
function undoAll() {
  S.run.quests = {};
  S.run.stop_after = null;
  S.run.start_kit = null;
  S.run.w2_import = false;
  S.idx = 0;
  refreshAfterUndo();
}
function closeApp() {
  if ((S.mode === 'edit' ? changeCount() : pendingCount()) && !window.confirm(t('close.confirm'))) return;
  try { if (TAURI) TAURI.window.getCurrentWindow().close(); else window.close(); } catch (_) { window.close(); }
}

function templateName() {
  const c = S.plan && S.plan.choice;
  if (S.game && S.run.template) return { name: S.run.template.split(/[\\/]/).pop().replace(/\.sav$/i, ''), stem: null, manual: true };
  if (c && c.template) return { name: c.template.id, stem: c.template.stem, line: c.template.line };
  return null;
}
function storyLevel() {
  const done = pendingItems().filter(x => x.e.status === 'completed' && (S.run.stop_after == null || x.i <= lastIdx()));
  return done.length ? done[done.length - 1].q.level : null;
}

function nav() {
  if (!S.book) return;
  if (S.mode === 'edit') return editNav(); // ui/edit.js
  const tpl = templateName();
  const lvl = storyLevel();
  const on = (...s) => s.includes(S.screen);

  // sidebar card
  document.getElementById('sidecard').replaceChildren(
    h('span', { class: 'eyebrow' }, tpl && tpl.manual ? t('side.template.manual') : t('side.template')),
    h('span', { class: 'name' + (tpl ? '' : ' muted') }, tpl ? tpl.name : t('side.template.none')),
    h('span', { class: 'meta' },
      lvl != null ? [h('span', { class: 'lvl' }, String(lvl)), ' ' + t('side.storylevel')] : t('side.noanswers'),
      tpl && tpl.line != null ? [h('span', null, '·'), ' ' + t('side.line', { n: tpl.line })] : null));

  // sidebar nav
  const item = (label, o) => h('button', {
    class: 'nav-item' + (o.on ? ' on' : ''), disabled: o.disabled, onclick: o.go, type: 'button',
  }, o.roman ? h('span', { class: 'roman' }, o.roman) : null, h('span', { class: 'label' }, label),
    o.count != null ? h('span', { class: 'count' + (o.full ? ' done' : '') }, String(o.count)) : null);
  const cur = S.screen === 'quest' || S.screen === 'checkpoint' ? curQuest() : null;
  const acts = actGroups().map(g => item(g.label, {
    roman: ROMAN[g.act] || String(g.act), count: `${g.done}/${g.idxs.length}`, full: g.done === g.idxs.length,
    on: cur && cur.act === g.act,
    disabled: g.idxs[0] > S.maxIdx + 1,
    go: () => { if (S.maxIdx < 0) return go('prologue'); S.idx = g.idxs[0]; S.step = 0; go('quest'); },
  }));
  document.getElementById('nav').replaceChildren(
    h('div', { class: 'nav-sec' }, h('div', { class: 'nav-label' }, t('nav.setup')),
      item(t('nav.templateplan'), { on: on('welcome', 'prologue'), go: () => go('welcome') })),
    h('div', { class: 'nav-sec' }, h('div', { class: 'nav-label' }, t('nav.story')), ...acts,
      item(t('nav.side'), { on: on('side'), count: sideCount(), go: () => go('side') })), // ui/side.js
    h('div', { class: 'nav-sec' }, h('div', { class: 'nav-label' }, t('nav.result')),
      item(t('nav.review'), { on: on('review'), count: pendingCount(), go: () => go('review') }),
      item(t('nav.startkit'), { on: false, go: () => { go('review'); setTimeout(() => { const k = document.getElementById('startkit'); if (k) { k.open = true; k.scrollIntoView({ block: 'start' }); } }, 60); } }),
      item(t('nav.export'), { on: on('export'), go: () => go('export') })));

  // top bar
  const n = pendingCount();
  document.getElementById('ctx').replaceChildren(
    h('span', { class: 'ctx-name' }, tpl ? tpl.name : t('top.notemplate')),
    h('span', { class: 'ctx-meta' }, lvl != null ? t('top.level', { n: lvl }) : '', tpl && tpl.stem ? [' · ', h('code', null, tpl.stem)] : null));
  document.getElementById('topactions').replaceChildren(
    h('span', { class: 'status-pill' + (n ? ' dirty' : '') }, n ? tn('top.unsaved', n) : t('top.nochanges')),
    h('button', { class: 'ghost', onclick: undoAll, disabled: !n }, t('top.undoall')),
    h('button', { class: 'primary', onclick: () => go('export'), disabled: S.screen === 'export' }, t('top.export')),
    h('button', { onclick: closeApp }, t('top.close')));

  // ledger
  const items = pendingItems();
  const kit = S.run.start_kit || {};
  const kitText = [kit.level != null ? t('kit.sum.level', { n: kit.level }) : null, kit.crowns != null ? t('kit.sum.crowns', { n: kit.crowns }) : null,
    kit.gear === 'recommended' ? t('kit.sum.supplies') : null, kit.new_game_plus === false ? t('kit.sum.ngplus') : null, kit.skill_points != null ? t('kit.sum.skill', { n: kit.skill_points }) : null].filter(Boolean).join(', ');
  document.getElementById('body').classList.toggle('no-ledger', S.screen === 'welcome' || S.screen === 'safety');
  document.getElementById('ledger').replaceChildren(
    h('div', { class: 'ledger-head' }, h('h3', null, t('ledger.title')),
      h('div', { class: 'sub' }, tn('ledger.answers', items.length))),
    items.length || kitText
      ? h('ul', { class: 'ledger-list' }, items.map(({ q, e }) => h('li', null,
          h('span', { class: 'q' }, qname(q)), h('span', { class: 'a' }, answerLine(q, e)),
          h('div', { class: 'row' }, h('span', { class: 'st' }, sealText(sealOf(q, e))),
            h('button', { class: 'link', onclick: () => undoQuest(q.id) }, t('ledger.undo'))))),
          kitText ? h('li', null, h('span', { class: 'q' }, t('ledger.kit')), h('span', { class: 'a' }, kitText),
            h('div', { class: 'row' }, h('span', { class: 'st' }, sealText('decided')),
              h('button', { class: 'link', onclick: () => { S.run.start_kit = null; refreshAfterUndo(); } }, t('ledger.undo')))) : null)
      : h('div', { class: 'ledger-empty' }, t('ledger.empty')),
    h('div', { class: 'ledger-foot' }, t('ledger.foot')));
}
function errText(e) {
  if (e && typeof e === 'object') return e.title ? `${e.title} ${e.detail || ''}` : (e.message || JSON.stringify(e));
  return String(e);
}

// ---------------------------------------------------------------- option list (keyboard + mouse)
function optionList(items, sel, confirm) {
  S.list = { items, sel, confirm };
  const ul = h('ul', { class: 'options', role: 'listbox' });
  items.forEach((it, i) => {
    ul.append(h('li', {
      class: i === sel ? 'sel' : '', role: 'option',
      onclick: () => { S.list.sel = i; paintList(); S.list.confirm(i); },
    }, h('span', { class: 'opt-text' }, it.text), it.def && h('span', { class: 'def' }, t('opt.default')),
    it.note && h('span', { class: 'note' + (it.unfinished ? ' unfinished' : '') }, it.note)));
  });
  S.listEl = ul;
  return ul;
}
function paintList() {
  [...S.listEl.children].forEach((li, i) => li.classList.toggle('sel', i === S.list.sel));
}
function moveSel(d) {
  if (!S.list) return;
  const n = S.list.items.length;
  S.list.sel = (S.list.sel + d + n) % n;
  paintList();
}

// ---------------------------------------------------------------- navigation
const SCREENS = { welcome, prologue, quest, checkpoint, review, export: exportScreen, safety, editlist: () => editlist(), editplan: () => editplan(), editresult: () => editresult(), side: () => sideScreen() }; // the edit* screens (ui/edit.js) and the side quests page (ui/side.js), loaded after this file
function go(screen) {
  S.screen = screen;
  S.list = null;
  nav();
  SCREENS[screen]();
}
const quests = () => S.book.quests;
const curQuest = () => quests()[S.idx];
function entryOf(id) { return S.run.quests[id]; }
function optionText(q, choiceId, optId) {
  const c = q.choices.find(c => c.id === choiceId);
  const o = c && c.options.find(o => o.id === optId);
  return o ? loc(o, 'text') : optId;
}

// ---------------------------------------------------------------- save summary
function worldLabel(w) {
  if (!w) return null;
  const stem = w.split(/[\\/]/).pop().replace(/\.w2w.*$/, '');
  return worldName(stem) ? `${worldName(stem)} (${stem})` : stem;
}
function versionLabel(g) {
  const ints = g.header_ints ? t('ver.header', { h: g.header_ints.join(' / ') }) : '';
  if (g.payload_error) return t('ver.notreadable', { e: g.payload_error });
  return g.looks_like_5x ? t('ver.ok', { h: ints }) : t('ver.unknown', { h: ints });
}
function saveSummary(g, compact) {
  if (!g) return null;
  const rows = [
    [t('save.file'), g.path],
    [t('save.version'), versionLabel(g)],
    [t('save.world'), worldLabel(g.world) || t('save.unknown')],
    [t('save.questlog'), g.journal_entries != null
      ? t('save.journal', { n: g.journal_entries }) + (g.journal_counters != null ? t('save.counters', { n: g.journal_counters }) : '')
      : (g.journal_error ? t('save.notreadable', { e: g.journal_error }) : t('save.unknown'))],
    [t('save.tracked'), g.tracked_quest || t('save.none')],
  ];
  if (!compact) rows.push([t('save.size'), t('save.size.val', { mb: (g.file_size / 1048576).toFixed(1), chunks: g.chunk_count }) + (g.fact_count != null ? t('save.facts', { n: g.fact_count }) : '')]);
  return h('div', null,
    h('table', { class: 'kv' }, h('tbody', null, rows.map(([k, v]) => h('tr', null, h('th', null, k), h('td', null, v))))),
    ...(g.issues || []).map(i => h('div', { class: i.severity === 'error' ? 'err-box' : 'warn-box' }, `${i.severity}: ${i.message}`)));
}

async function setTemplate(p) {
  S.game = await invoke('detect_game', { savePath: p });
  S.run.template = p;
  const bad = S.game.issues.some(x => x.severity === 'error') || S.game.payload_error;
  banner(bad ? 'error' : 'ok', bad ? t('tpl.bad') : t('tpl.ok'));
}

// The automatic template choice (questbook/templates.toml; docs/plan/story-chain.md).
async function refreshPlan() {
  try { S.plan = await invoke('template_plan', { run: S.run }); } catch (e) { S.plan = { dirs: [], choice: null, error: errText(e), templates: [] }; }
}
function templatesFound() {
  return !!(S.plan && S.plan.templates && S.plan.templates.some(t => t.found));
}
const answersText = m => Object.keys(m || {}).length ? Object.entries(m).map(([c, o]) => `${c}: ${o}`).join(', ') : t('plan.done');
function planView(welcomeScreen) {
  const p = S.plan;
  if (!p) return h('p', { class: 'small muted' }, t('plan.looking'));
  const found = (p.templates || []).filter(x => x.found).length;
  const folders = h('p', { class: 'small muted' }, t('plan.folders', { dirs: (p.dirs || []).join('; ') || t('plan.none'), found, total: (p.templates || []).length }));
  const c = p.choice;
  if (!c) {
    const why = welcomeScreen && !Object.keys(S.run.quests).length
      ? t('plan.why.start')
      : t('plan.why.nofit', { e: p.error || t('plan.unknown') });
    return h('div', null, h('p', { class: 'small' + (found ? ' muted' : '') }, why), folders,
      found ? null : h('div', { class: 'warn-box' }, t('plan.notfound')));
  }
  const rows = [];
  for (const b of c.baked || []) {
    if (b.action === 'rewrite') rows.push(h('li', null, t('plan.rewrite', { q: questTitle(b.quest), from: answersText(b.from), to: answersText(b.to) })));
    else if (b.action === 'fixed') rows.push(h('li', { class: 'bad' }, t('plan.fixed', { q: questTitle(b.quest), why: b.why })));
  }
  const nRewrite = (c.baked || []).filter(b => b.action === 'rewrite').length;
  return h('div', null,
    h('div', { class: 'plan-grid' },
      h('div', { class: 'tile' }, h('span', { class: 'eyebrow' }, t('plan.tile.tpl')),
        h('span', { class: 'big' }, c.template.id),
        h('span', { class: 'small muted' }, t('plan.tile.line', { n: c.template.line }), h('code', null, c.path || t('plan.notfoundpath')))),
      h('div', { class: 'tile' }, h('span', { class: 'eyebrow' }, t('plan.tile.quests')),
        h('span', { class: 'big' }, String((c.applied || []).length), h('span', { class: 'of' }, S.book ? ` / ${quests().length}` : '')),
        h('span', { class: 'small muted' }, (c.applied || []).map(questTitle).join(', ') || t('plan.none'))),
      h('div', { class: 'tile' }, h('span', { class: 'eyebrow' }, t('plan.tile.rewrites')),
        h('span', { class: 'big' }, String(nRewrite)),
        h('span', { class: 'small muted' }, t('plan.tile.rewrites.sub')))),
    h('div', { style: 'height:16px' }),
    rows.length ? h('ul', { class: 'plain small' }, rows) : h('p', { class: 'small muted' }, t('plan.allsame')),
    (c.beyond || []).length ? h('div', { class: 'callout' }, t('plan.beyond', { list: c.beyond.map(questTitle).join(', ') })) : null,
    ...(c.passed_over || []).map(x => h('p', { class: 'small muted' }, t('plan.passed', { x }))),
    folders);
}
function questTitle(id) {
  const q = S.book && S.book.quests ? S.book.quests.find(x => x.id === id) : null;
  return q ? qname(q) : id;
}

// ---------------------------------------------------------------- screens
function welcome() {
  hints(['↑↓', 'hint.select'], ['Enter', 'hint.confirm'], ['T', 'hint.template'], ['D', 'hint.donors'], ['L', 'hint.loadrun']);
  const g = S.game;
  const started = S.maxIdx >= 0 || Object.keys(S.run.quests).length > 0;

  const donorList = S.run.donors.length
    ? h('ul', { class: 'plain' }, S.run.donors.map((d, i) => h('li', null,
        h('code', null, d), ' ',
        h('button', { class: 'link', onclick: () => { S.run.donors.splice(i, 1); welcome(); } }, t('welcome.donor.remove')))))
    : h('p', { class: 'small muted' }, t('welcome.donors.none'));

  const auto = templatesFound();
  // Optional setup, folded away under the main button: everything works without it (shipped templates, no donors).
  const items = [
    { text: g ? t('welcome.opt.another') : t('welcome.opt.byhand'), note: g ? t('welcome.note.manual') : (auto ? t('welcome.note.auto') : t('welcome.note.none')) },
    { text: t('welcome.opt.donors'), note: S.run.donors.length ? t('welcome.note.folders', { n: S.run.donors.length }) : t('welcome.note.optional') },
    { text: t('welcome.opt.load'), note: t('welcome.note.load') },
    { text: t('welcome.opt.tdir'), note: S.run.template_dirs.length ? S.run.template_dirs.join('; ') : t('welcome.note.tdir') },
  ];
  const ACTION = [0, 1, 2, 4]; // welcomeAction indices of the items above (3 = begin)
  const begin = h('button', { class: 'primary big begin', onclick: () => welcomeAction(3, started) },
    started ? t('welcome.opt.continue') : t('welcome.opt.begin'));
  mount(h('div', { class: 'page' },
    h('div', { class: 'hero' },
      h('div', { class: 'page-head' }, h('h1', null, t('hero.title')), h('span', { class: 'pill safe' }, t('pill.safe'))),
      h('p', { class: 'sub' }, t('hero.sub')),
      h('div', { class: 'begin-row' }, begin,
        g || auto ? null : h('span', { class: 'small muted' }, t('welcome.note.storyonly'))),
      h('p', { class: 'small muted' }, t('hero.disclaimer'))),
    modeSwitch(), // ui/edit.js
    h('div', { class: 'callout' }, h('b', null, t('callout.close')), t('callout.close2')),
    h('details', { class: 'more' },
      h('summary', null, t('welcome.more')),
      h('div', { class: 'panel' },
        h('h3', null, t('panel.template')),
        g ? h('div', null, saveSummary(g), h('button', { class: 'link', onclick: () => { S.run.template = null; S.game = null; refreshPlan().then(welcome); } }, t('welcome.autochoice')))
          : h('div', null, planView(true))),
      h('div', { class: 'panel' }, h('h3', null, t('panel.donors')), donorList),
      h('div', { class: 'panel' }, optionList(items, -1, i => welcomeAction(ACTION[i], started))))));
  begin.focus();
}

async function welcomeAction(i, started) {
  try {
    if (i === 0) {
      const p = await pickFile([{ name: t('pick.save'), extensions: ['sav'] }]);
      if (p) await setTemplate(p);
      welcome();
    } else if (i === 1) {
      const d = await pickDir(t('pick.donordir'));
      if (d && !S.run.donors.includes(d)) S.run.donors.push(d);
      S.donors = null;
      welcome();
    } else if (i === 2) {
      const p = await pickFile([{ name: t('pick.run'), extensions: ['toml'] }]);
      if (p) await openRun(p);
    } else if (i === 4) {
      const d = await pickDir(t('pick.tpldir'));
      if (d) { S.run.template_dirs = [d]; await refreshPlan(); }
      welcome();
    } else if (!S.book.quests.length) {
      banner('warn', t('welcome.noquests'));
    } else if (started) {
      S.step = 0; go('quest');
    } else go('prologue');
  } catch (e) { banner('error', errText(e)); }
}

function safety() {
  hints(['Esc', 'hint.back']);
  S.safetyFrom = S.safetyFrom || 'welcome';
  mount(h('div', { class: 'page' },
    h('div', { class: 'page-head' }, h('h1', null, t('safety.title')), h('span', { class: 'pill safe' }, t('safety.pill'))),
    h('p', { class: 'lede' }, t('safety.lede')),
    h('div', { class: 'panel' }, h('h3', null, t('safety.h.saves')),
      h('ul', { class: 'plain' },
        h('li', null, t('safety.l1')),
        h('li', null, t('safety.l2')),
        h('li', null, t('safety.l3a'), h('code', null, 'Documents\\The Witcher 3\\gamesaves'), t('safety.l3b')),
        h('li', null, t('safety.l4')))),
    h('div', { class: 'panel' }, h('h3', null, t('safety.h.limits')),
      h('ul', { class: 'plain' },
        h('li', null, t('safety.lim1')),
        h('li', null, t('safety.lim2')),
        h('li', null, t('safety.lim3')),
        h('li', null, t('safety.lim4')),
        h('li', null, t('safety.lim5')))),
    h('button', { onclick: () => go(S.safetyFrom) }, t('safety.back'))));
}

async function openRun(path) {
  const run = await invoke('load_run', { path });
  S.run = Object.assign(newRun(), run, { donors: run.donors || [], template_dirs: run.template_dirs || [] });
  S.plan = null;
  if (!S.run.template) await refreshPlan();
  S.maxIdx = -1;
  quests().forEach((q, i) => { if (S.run.quests[q.id]) S.maxIdx = i; });
  S.idx = Math.max(0, S.maxIdx);
  S.game = null;
  S.donors = null;
  S.exportResult = null;
  if (S.run.template) {
    try { S.game = await invoke('detect_game', { savePath: S.run.template }); } catch (e) {
      banner('warn', t('run.loadfail', { path, e: errText(e) }));
      return go('review');
    }
  }
  banner('ok', t('run.loaded', { path }));
  go('review');
}

function prologue() {
  hints(['↑↓', 'hint.select'], ['Enter', 'hint.confirm'], ['Esc', 'hint.back']);
  const items = [
    { text: t('prologue.opt1'), note: t('prologue.note1'), def: true },
    { text: t('prologue.opt2'), note: t('prologue.note2') },
  ];
  mount(h('div', { class: 'page story' },
    h('div', { class: 'story-top' }, h('div', { class: 'where' }, t('prologue.where'))),
    h('h1', null, t('prologue.h1')),
    h('p', { class: 'narration' }, t('prologue.narration')),
    optionList(items, S.run.w2_import ? 1 : 0, i => { S.run.w2_import = i === 1; S.idx = 0; S.step = 0; go('quest'); })));
}

const statusItems = () => ['completed', 'started', 'skipped'].map(s => ({ status: s, text: t('status.' + s), note: t('status.' + s + '.note') }));

function quest() {
  if (!quests().length) return go('review');
  S.idx = Math.max(0, Math.min(S.idx, quests().length - 1));
  const q = curQuest();
  const choices = q.choices || [];
  S.maxIdx = Math.max(S.maxIdx, S.idx);
  const e = entryOf(q.id);
  hints(['↑↓', 'hint.select'], ['Enter', 'hint.confirm'], ['Esc', 'hint.back'], ['S', 'hint.stophere'], ['R', 'hint.review']);

  const bar = h('div', { class: 'progress' }, h('i'));
  bar.firstChild.style.width = ((S.idx / quests().length) * 100) + '%';

  let body;
  if (S.step === 0 || !choices[S.step - 1]) {
    S.step = 0;
    const items = statusItems();
    const sel = Math.max(0, items.findIndex(s => e && s.status === e.status));
    body = [
      h('div', { class: 'prompt' }, t('quest.what')),
      choices.length ? h('p', { class: 'hint-line' }, tn('quest.decisions', choices.length))
        : h('p', { class: 'hint-line' }, t('quest.nodecisions')),
      q.kind === 'main' ? h('p', { class: 'hint-line' }, t('quest.mainwarn')) : null,
      optionList(items, sel, i => {
        const status = items[i].status;
        S.run.quests[q.id] = { status, answers: (e && e.answers) || {} };
        if (status === 'completed' && choices.length) { S.step = 1; quest(); } else advance();
      }),
    ];
  } else {
    const c = choices[S.step - 1];
    const answers = (e && e.answers) || {};
    const cur = answers[c.id] || c.default;
    const items = c.options.map(o => ({ text: loc(o, 'text'), def: o.id === c.default, ...effectNote(q, o.effects) }));
    body = [
      h('div', { class: 'step-of' }, t('quest.decision', { i: S.step, n: choices.length })),
      h('div', { class: 'prompt' }, loc(c, 'prompt')),
      optionList(items, Math.max(0, c.options.findIndex(o => o.id === cur)), i => {
        const entry = S.run.quests[q.id] || (S.run.quests[q.id] = { status: 'completed', answers: {} });
        entry.answers = entry.answers || {};
        entry.answers[c.id] = c.options[i].id;
        if (S.step < choices.length) { S.step++; quest(); } else advance();
      }),
    ];
  }

  const warn = S.book.warnings[q.id];
  const art = illustration(q);
  mount(h('div', { class: 'story' },
    h('div', { class: 'story-top' },
      h('div', { class: 'where' }, t('quest.where', { act: ROMAN[q.act] || q.act, region: regionName(q.region), i: S.idx + 1, n: quests().length })),
      h('div', { class: 'row' },
        h('button', { class: 'sm', onclick: stopHere, disabled: S.idx === 0 && S.step === 0 }, t('quest.stop')),
        h('button', { class: 'sm', onclick: () => go('review') }, t('quest.review')))),
    bar,
    h('div', { class: 'story-grid' + (art ? (art.wide ? ' wide' : '') : ' no-art') },
      art && art.node,
      h('div', null,
        h('h1', null, qname(q)),
        h('div', { class: 'tags' },
          h('span', { class: 'pill gold' }, regionName(q.region)),
          h('span', { class: 'pill' }, t('quest.level', { n: q.level })),
          h('span', { class: 'pill' }, t.has('kind.' + q.kind) ? t('kind.' + q.kind) : q.kind)),
        warn ? h('div', { class: 'callout' }, warn) : null,
        h('p', { class: 'narration' }, loc(q, 'summary')),
        gameDescription(q),
        ...body))));
}

// The quest's own journal text from the player's install (GameText, ui/gametext.js); nothing when there is none.
function gameDescription(q) {
  const d = typeof GameText !== 'undefined' && GameText.description(q, 0);
  if (!d) return null;
  return h('div', { class: 'game-desc' },
    h('div', { class: 'muted sm' }, t('quest.gamejournal')),
    h('p', null, d));
}

// ---------------------------------------------------------------- illustrations
// Linked, not stored (DECISIONS.md): URLs from questbook/images.toml on the Witcher Wiki CDN, shown with a credit.
// The switch is in the sidebar; the choice lives in localStorage (it may be unavailable: then it is just on).
const IMG_KEY = 'w3al.images';
function imagesOn() {
  try { return window.localStorage.getItem(IMG_KEY) !== 'off'; } catch (_) { return true; }
}
function setImagesOn(on) {
  try { window.localStorage.setItem(IMG_KEY, on ? 'on' : 'off'); } catch (_) { /* lasts this session only */ }
}
function openWiki(ev, url) {
  if (!TAURI) return; // browser preview: the plain link opens in a new tab
  ev.preventDefault();
  invoke('open_wiki_page', { url }).catch(() => {});
}
// Lookup: our quest id, then the quest's journal GUID (every game quest, incl. generated side quests), then region, then act.
function imageLink(q) {
  if (!imagesOn()) return null;
  const jg = q.journal_guid || (typeof GameText !== 'undefined' && GameText.guid(q));
  return S.images[q.id] || (jg && S.images['journal:' + jg]) || S.images['region:' + q.region] || S.images['act:' + q.act] || null;
}
// The banner is at most ~900px wide, so ask the wiki CDN for 800px instead of 1000px (smaller files, faster).
// Inside the desktop app the image comes from the Rust disk cache through the `wikiimg` scheme (WebView2 and
// Android use http://wikiimg.localhost/, other platforms wikiimg://localhost/); the browser preview uses the CDN.
const WIKIIMG_BASE = /Windows|Android/i.test(navigator.userAgent) ? 'http://wikiimg.localhost/' : 'wikiimg://localhost/';
const wikiSrc = url => {
  const u = url.replace('/scale-to-width-down/1000', '/scale-to-width-down/800');
  return TAURI ? WIKIIMG_BASE + encodeURIComponent(u) : u;
};
// One <img> per URL for the whole session: a repaint reuses the loaded element instead of fetching again.
const IMG_CACHE = new Map();
function wikiImage(url) {
  let img = IMG_CACHE.get(url);
  if (!img) {
    img = new Image();
    img.decoding = 'async';
    img.referrerPolicy = 'no-referrer';
    img.src = wikiSrc(url);
    IMG_CACHE.set(url, img);
  }
  return img;
}
// Warm the cache for every questbook quest once, a few at a time, in the background. The wiki CDN answers in
// 1-3 s per image (much longer the first time a size is asked for) and sends max-age one year, so the webview's
// own HTTP cache keeps them for later sessions.
// Always 12 requests in flight at once (12 workers; each starts the next URL as soon as its image arrives):
// first the questbook's quests, then every other linked image (all game quests, regions, acts).
function warmImages() {
  if (!imagesOn() || !S.book) return;
  // The Rust side downloads every linked image into its disk cache at startup; this only loads the questbook's own.
  const urls = [...new Set(S.book.quests.map(imageLink).filter(Boolean).map(l => l.image))];
  let next = 0;
  const pump = () => {
    if (next >= urls.length) return;
    const img = wikiImage(urls[next++]);
    if (img.complete) pump();
    else { img.addEventListener('load', pump, { once: true }); img.addEventListener('error', pump, { once: true }); }
  };
  for (let k = 0; k < 12; k++) pump();
}
// Start fetching the next few quests' images while the player reads this one.
function prefetchImages(q) {
  const list = (S.book && S.book.quests) || [];
  const i = list.indexOf(q);
  for (const n of list.slice(i + 1, i + 13)) { const l = imageLink(n); if (l) wikiImage(l.image); }
}

// { node, wide } for the story page's image slot, or null when there is nothing to show.
function illustration(q) {
  const link = imageLink(q);
  const local = !link && q.image ? q.image : null;
  if (!link && !local) return null;
  const col = h('div', { class: 'art-col' + (link ? ' wide' : '') });
  const panel = h('div', { class: 'art' });
  const fail = () => { // offline or a dead link: the plain panel with the region name, no credit
    col.classList.remove('wide');
    col.closest('.story-grid')?.classList.remove('wide');
    panel.classList.remove('loading');
    panel.replaceChildren(document.createTextNode(regionName(q.region)));
    credit && credit.remove();
  };
  let img;
  if (link) {
    img = wikiImage(link.image);
    img.alt = t('img.alt', { title: qname(q) });
    img.className = 'wiki';
    if (img.complete && img.naturalWidth === 0) setTimeout(fail);
    else if (!img.complete) {
      panel.classList.add('loading');
      img.onload = () => panel.classList.remove('loading');
      img.onerror = fail;
    }
    prefetchImages(q);
  } else {
    img = h('img', { src: local, alt: '', decoding: 'async', onerror: fail });
  }
  panel.append(img);
  col.append(panel);
  let credit = null;
  if (link) {
    credit = h('div', { class: 'art-credit' },
      t('img.credit') + ' ',
      h('a', { href: link.page, target: '_blank', rel: 'noopener noreferrer', title: link.title, onclick: ev => openWiki(ev, link.page) }, t('img.credit.link')));
    col.append(credit);
  }
  return { node: col, wide: !!link };
}
function imagesToggle() {
  const box = h('input', { type: 'checkbox', id: 'img-toggle', checked: imagesOn(), onchange: ev => { setImagesOn(ev.target.checked); if (S.book && SCREENS[S.screen]) go(S.screen); } });
  document.getElementById('img-slot').replaceChildren(h('label', { class: 'img-toggle', for: 'img-toggle', title: t('img.setting.hint') }, box, h('span', null, t('img.setting'))));
}

// What an option writes, in plain words, for the choice buttons.
function effectNote(q, key) {
  const e = key && q.effects ? q.effects[key] : null;
  if (!e) return {};
  const ph = e.placeholder || JSON.stringify(e).includes('PLACEHOLDER_');
  if (ph) return { note: t('fx.unfinished'), unfinished: true };
  const parts = [];
  const nf = Object.keys(e.facts || {}).length + (e.facts_remove || []).length;
  if (nf) parts.push(tn('fx.facts', nf));
  const nj = (e.journal_status || []).length + (e.journal_append || []).length + (e.journal_counters || []).length + (e.journal_track ? 1 : 0);
  if (nj) parts.push(tn('fx.log', nj));
  if (e.graph) parts.push(t('fx.graph'));
  return parts.length ? { note: t('fx.writes', { list: parts.join(', ') }) } : { note: t('fx.nothing') };
}

function checkpoint() {
  const q = curQuest();
  const last = S.idx === quests().length - 1;
  hints(['↑↓', 'hint.select'], ['Enter', 'hint.confirm'], ['Esc', 'hint.back'], ['S', 'hint.wake'], ['R', 'hint.review']);
  const items = last ? [{ text: t('checkpoint.wake'), note: t('checkpoint.wake.end') }] : [{ text: t('welcome.opt.continue') }, { text: t('checkpoint.wake'), note: t('checkpoint.wake.freeze') }];
  mount(h('div', { class: 'page story' },
    h('div', { class: 'story-top' }, h('div', { class: 'where' }, t('checkpoint.after', { title: qname(q) }))),
    h('h1', null, t('checkpoint.h1')),
    h('div', { style: 'height:16px' }),
    optionList(items, 0, i => {
      if (last || i === 1) { wakeUp(); return; }
      S.run.stop_after = null; S.idx++; S.step = 0; go('quest');
    })));
}

// After a quest: straight on to the next one (the "Stop here" button on every quest page ends the story);
// after the last quest: the review.
function advance() {
  if (S.idx >= quests().length - 1) { wakeUp(); return; }
  S.run.stop_after = null; S.idx++; S.step = 0; go('quest');
}

function wakeUp() {
  S.run.stop_after = curQuest().id;
  go('review');
}

function stopHere() {
  if (S.screen === 'checkpoint') return wakeUp();
  if (S.screen !== 'quest') return;
  // Mid-quest (status chosen, answering decisions): the quest counts with its current answers.
  if (S.step > 0) return wakeUp();
  if (S.idx === 0) { banner('warn', t('stop.nothing')); return; }
  S.run.stop_after = quests()[S.idx - 1].id;
  go('review');
}

function lastIdx() {
  if (S.run.stop_after) return quests().findIndex(q => q.id === S.run.stop_after);
  return S.maxIdx;
}

async function runValidation() {
  try { S.validation = await invoke('validate_run', { run: S.run }); } catch (e) { S.validation = { error: errText(e) }; }
}

async function review() {
  hints(['Esc', 'hint.backstory'], ['E', 'hint.export'], ['R', 'hint.review']);
  await runValidation();
  if (S.screen !== 'review') return;
  const regions = ['all', ...new Set(quests().map(q => q.region))];
  const last = lastIdx();
  const v = S.validation;
  const blocked = new Set((v && v.blocked) || []);

  // One entry per quest in the book; quests after the stopping point are "not reached".
  const all = quests().map((q, i) => {
    const e = i <= last ? entryOf(q.id) : null;
    return { q, i, e, seal: e ? sealOf(q, e) : 'notreached', blocked: blocked.has(q.id) };
  }).filter(x => S.filter === 'all' || x.q.region === S.filter);
  const tabs = [['all', t('review.tab.all')], ['decided', sealText('decided')], ['default', sealText('default')], ['notreached', sealText('notreached')]];
  const tab = S.revTab || 'all';
  const shown = all.filter(x => tab === 'all' || x.seal === tab);

  const setAnswer = (q, c, optId) => {
    const ent = S.run.quests[q.id] || (S.run.quests[q.id] = { status: 'completed', answers: {} });
    ent.answers = ent.answers || {};
    ent.answers[c.id] = optId;
    S.exportResult = null;
    review();
  };
  const rows = shown.map(({ q, i, e, seal, blocked: isBlocked }) => {
    const done = e && e.status === 'completed';
    const alts = done ? (q.choices || []).flatMap(c => {
      const cur = (e.answers || {})[c.id] || c.default;
      return c.options.filter(o => o.id !== cur).map(o => h('button', { type: 'button', onclick: () => setAnswer(q, c, o.id) }, loc(o, 'text')));
    }) : [];
    const facts = [];
    if (done) for (const c of q.choices || []) {
      const o = c.options.find(o => o.id === ((e.answers || {})[c.id] || c.default));
      const blk = o && o.effects && q.effects ? q.effects[o.effects] : null;
      if (blk) for (const [k, val] of Object.entries(blk.facts || {})) facts.push(h('li', null, h('code', null, k), ' = ' + val));
    }
    const dn = q.effects && q.effects[q.done_effects];
    if (done && dn) for (const [k, val] of Object.entries(dn.facts || {})) facts.push(h('li', null, h('code', null, k), ' = ' + val));
    return h('li', { class: 'tl' }, h('span', { class: 'seal ' + seal, title: sealText(seal) }),
      h('div', null,
        h('div', { class: 'tl-head' },
          h('span', { class: 'tl-title' }, qname(q)),
          h('span', { class: 'tl-where' }, t('review.where', { act: ROMAN[q.act] || q.act, region: regionName(q.region), n: q.level })),
          isBlocked ? h('span', { class: 'st st-blocked' }, t('review.blocked')) : null,
          e && !isBlocked && e.status !== 'completed' ? h('span', { class: 'st st-' + e.status }, t('st.' + e.status)) : null,
          i <= last ? h('button', { class: 'link small', onclick: () => { S.idx = i; S.step = 0; go('quest'); } }, t('review.open')) : null),
        h('div', { class: 'outcome' + (seal === 'decided' ? '' : ' dim') }, seal === 'notreached' ? sealText('notreached') : answerLine(q, e || { status: 'skipped' })),
        alts.length ? h('div', { class: 'alts' }, alts) : null,
        facts.length ? h('details', { class: 'facts' }, h('summary', null, t('review.details')), h('ul', { class: 'plain small' }, facts)) : null));
  });

  const diag = v && v.diagnostics ? v.diagnostics.filter(d => d.severity === 'error') : [];
  const stopQuest = last >= 0 ? quests()[last] : null;
  mount(h('div', { class: 'page' },
    h('div', { class: 'page-head' }, h('h1', null, S.run.stop_after ? t('review.h1.wake') : t('review.h1')), h('span', { class: 'pill caution' }, t('review.pill'))),
    h('p', { class: 'lede' }, S.run.stop_after && stopQuest
      ? t('review.lede.stop', { title: qname(stopQuest) })
      : t('review.lede')),
    v && v.error ? h('div', { class: 'callout' }, v.error) : h('p', { class: 'small muted' },
      t('review.counts', { c: v.completed.length, s: v.started.length, b: v.blocked.length, e: v.effects.length, w: S.run.w2_import ? t('review.w2.sim') : t('review.w2.def') })),
    ...diag.map(d => h('div', { class: 'err-box' }, `${d.quest || ''} ${d.message}`)),
    startKitCard(stopQuest),
    h('div', { class: 'row spread', style: 'margin-bottom:16px' },
      h('div', { class: 'tabs', style: 'margin:0' }, tabs.map(([k, label]) => h('button', {
        type: 'button', class: 'tab' + (k === tab ? ' on' : ''), onclick: () => { S.revTab = k; review(); },
      }, label, h('span', { class: 'n' }, String(all.filter(x => k === 'all' || x.seal === k).length))))),
      h('div', { class: 'row' },
        h('select', { 'aria-label': t('review.region'), onchange: ev => { S.filter = ev.target.value; review(); } },
          regions.map(r => h('option', { value: r, selected: r === S.filter }, r === 'all' ? t('review.allregions') : regionName(r)))),
        h('button', { onclick: saveRun }, t('review.saverun')),
        h('button', { class: 'primary', onclick: () => go('export') }, t('review.export')))),
    rows.length ? h('ol', { class: 'timeline' }, rows) : h('p', { class: 'muted' }, t('review.empty'))));
}

async function saveRun() {
  try {
    const p = await pickSavePath('another_life.run.toml');
    if (!p) return;
    await invoke('save_run', { path: p, run: S.run });
    banner('ok', t('run.saved', { p }));
  } catch (e) { banner('error', errText(e)); }
}

// ---------------------------------------------------------------- start kit
// Optional: level, crowns and supplies written into the exported save ([start_kit] in the run file).
// Proven offline only; see docs/findings/character-inventory.md.
function startKitCard(stopQuest) {
  const kit = S.run.start_kit || {};
  const recommended = stopQuest && stopQuest.level ? Number(stopQuest.level) : null;
  const set = (k, v) => {
    const next = Object.assign({}, S.run.start_kit || {});
    if (v === null || v === '' || Number.isNaN(v)) delete next[k]; else next[k] = v;
    if (next.gear === 'none') delete next.gear;
    S.run.start_kit = Object.keys(next).length ? next : null;
  };
  const num = (k, min, max, ph) => h('input', {
    type: 'number', min, max, value: kit[k] == null ? '' : kit[k], placeholder: ph, 'data-kit': k,
    onchange: ev => { set(k, ev.target.value === '' ? null : Math.max(min, Math.min(max, parseInt(ev.target.value, 10)))); nav(); }
  });
  // [-] value [+]; an empty field means "keep what the save has".
  const stepper = (k, min, max, ph, step, from) => {
    const input = num(k, min, max, ph);
    const bump = d => () => {
      const cur = input.value === '' ? from : parseInt(input.value, 10);
      const nv = Math.max(min, Math.min(max, (Number.isNaN(cur) ? from : cur) + d));
      input.value = nv; set(k, nv); nav();
    };
    return h('span', { class: 'stepper' }, h('button', { type: 'button', 'aria-label': t('kit.less'), onclick: bump(-step) }, '−'), input,
      h('button', { type: 'button', 'aria-label': t('kit.more'), onclick: bump(step) }, '+'));
  };
  return h('details', { class: 'panel start-kit', id: 'startkit', open: kitCount() > 0 },
    h('summary', null, h('span', { class: 'eyebrow' }, t('kit.title'))),
    h('p', { class: 'small muted', style: 'margin-top:12px' }, t('kit.intro')),
    h('div', { class: 'kit-grid' },
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.level')),
        stepper('level', 1, 50, recommended ? String(recommended) : t('kit.keep'), 1, recommended || 1),
        h('span', { class: 'was' }, recommended ? [t('kit.level.story', { n: recommended }), h('button', { class: 'link', onclick: () => { set('level', recommended); review(); } }, t('kit.use'))] : t('kit.level.keep'))),
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.crowns')),
        stepper('crowns', 0, 999999, t('kit.keep'), 100, 0),
        h('span', { class: 'was' }, t('kit.crowns.keep'))),
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.skill')),
        stepper('skill_points', 0, 300, t('kit.keep'), 1, kit.level ? kit.level - 1 : 0),
        h('span', { class: 'was' }, kit.level ? [t('kit.skill.level', { lvl: kit.level, n: kit.level - 1 }), h('button', { class: 'link', onclick: () => { set('skill_points', kit.level - 1); review(); } }, t('kit.use'))] : t('kit.skill.keep'))),
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.supplies')),
        h('select', { 'aria-label': t('kit.supplies'), onchange: ev => { set('gear', ev.target.value); nav(); } },
          h('option', { value: 'none', selected: !kit.gear || kit.gear === 'none' }, t('kit.keepasis')),
          h('option', { value: 'recommended', selected: kit.gear === 'recommended' }, t('kit.recommended')))),
      // Every shipped template comes from a New Game+ playthrough (docs/findings/new-game-plus.md).
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.ngplus')),
        h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: kit.new_game_plus === false,
          onchange: ev => { set('new_game_plus', ev.target.checked ? false : null); nav(); } }), t('kit.ngplus.off')),
        h('span', { class: 'was' }, t('kit.ngplus.note')))));
}

// ---------------------------------------------------------------- export
function effectLine(a) {
  const e = a.effect;
  const parts = [];
  const nFacts = Object.keys(e.facts || {}).length;
  if (nFacts) parts.push(tn('fx.facts', nFacts));
  const nJournal = (e.journal_status || []).length + (e.journal_append || []).length + (e.journal_counters || []).length + (e.journal_track ? 1 : 0);
  if (nJournal) parts.push(tn('fx.log', nJournal));
  if (e.graph) parts.push(tn('efx.ops', e.graph.ops.length) + (e.graph.donor ? t('efx.from', { d: e.graph.donor }) : ''));
  if (e.synth) parts.push(t('efx.synth'));
  if (Object.keys(e.quests || {}).length || (e.journal || []).length) parts.push(t('efx.legacy'));
  const ph = e.placeholder || JSON.stringify(e).includes('PLACEHOLDER_');
  return h('li', { class: ph ? 'bad' : '' }, `${questTitle(a.quest)} / ${a.origin}: ${parts.join(', ') || t('efx.nothing')}`, ph ? t('efx.ph') : '');
}

function donorRows() {
  if (!S.run.donors.length && !(S.donors && S.donors.length)) {
    return h('p', { class: 'small muted' }, t('donor.none'));
  }
  return h('div', null,
    h('div', { class: 'small' }, t('donor.folders'), S.run.donors.length ? S.run.donors.join('; ') : t('plan.none')),
    S.donors == null ? h('p', { class: 'small muted' }, t('donor.checking'))
      : !S.donors.length ? h('p', { class: 'small muted' }, t('donor.nocopy'))
      : h('ul', { class: 'plain small' }, S.donors.map(d => h('li', { class: d.found ? 'good' : (d.needed ? 'bad' : 'muted') },
          `${d.stem}.sav — `,
          d.found ? t('donor.found', { f: d.found }) : (d.needed ? t('donor.missing', { p: d.problem }) : t('donor.notneeded')),
          h('span', { class: 'muted' }, t('donor.usedby', { q: d.used_by.join(', ') }))))));
}

function resultView(r) {
  if (!r) return null;
  if (r.fail) {
    const f = r.fail;
    return h('div', { class: 'result fail' },
      h('h2', null, f.title),
      f.hint ? h('p', null, f.hint) : null,
      f.detail ? h('pre', { class: 'out' }, f.detail) : null);
  }
  const ok = r.ok;
  return h('div', { class: 'result ok' },
    h('h2', null, ok.written ? t('res.complete') : t('res.checked')),
    h('p', null, tn('res.applied', ok.effects_applied)),
    ok.notes.length ? h('ul', { class: 'plain small' }, ok.notes.map(n => h('li', null, n))) : h('p', { class: 'small muted' }, t('res.none')),
    h('p', { class: 'verified' }, ok.verified),
    ok.written ? h('div', null,
      h('p', null, t('res.written')),
      h('ul', { class: 'plain small' }, [ok.sav, ...ok.sidecars].map(f => h('li', null, h('code', null, f)))),
      h('p', { class: 'small muted' }, t('res.ingame')))
      : h('p', { class: 'small muted' }, t('res.notwritten')));
}

async function exportScreen() {
  hints(['C', 'hint.check'], ['E', 'hint.export'], ['F', 'hint.folder'], ['N', 'hint.remap'], ['Esc', 'hint.back']);
  if (!S.exportDir && S.paths && S.paths.gamesaves) S.exportDir = S.paths.gamesaves;
  paintExport();
  // Refresh what depends on the run, then repaint.
  await runValidation();
  try { S.donors = await invoke('donor_status', { run: S.run }); } catch (e) { S.donors = []; banner('error', errText(e)); }
  if (!S.run.template) await refreshPlan();
  if (S.screen === 'export') paintExport();
}

function paintExport() {
  const v = S.validation;
  const g = S.game;
  const isGameDir = S.paths && S.exportDir === S.paths.gamesaves;
  const effects = v && v.effects ? v.effects : null;
  const errors = v && v.diagnostics ? v.diagnostics.filter(d => d.severity === 'error') : [];

  const nPlaceholder = effects ? effects.filter(a => a.effect.placeholder || JSON.stringify(a.effect).includes('PLACEHOLDER_')).length : 0;
  mount(h('div', { class: 'page' },
    h('div', { class: 'page-head' }, h('h1', null, t('export.title')), h('span', { class: 'pill expert' }, t('export.pill'))),
    h('p', { class: 'lede' }, t('export.lede')),
    nPlaceholder ? h('div', { class: 'callout red' }, h('b', null, tn('export.unfinished', nPlaceholder)), t('export.unfinished.rest')) : null,

    h('div', { class: 'panel' }, h('h3', null, t('export.h.template')),
      g ? saveSummary(g, true) : planView(false)),

    h('div', { class: 'panel' }, h('h3', null, t('panel.donors')), donorRows()),

    h('div', { class: 'panel' }, h('h3', null, t('export.h.change')),
      !effects ? h('p', { class: 'small muted' }, v && v.error ? v.error : t('export.resolving'))
        : effects.length ? h('ul', { class: 'plain small' }, effects.map(effectLine))
        : h('p', { class: 'small muted' }, t('export.noeffects')),
      ...errors.map(d => h('div', { class: 'err-box' }, `${d.quest || ''} ${d.message}`))),

    h('div', { class: 'panel' }, h('h3', null, t('export.h.options')),
      h('label', { class: 'check' },
        h('input', { type: 'checkbox', id: 'remap', checked: S.run.allow_name_remap, onchange: ev => { S.run.allow_name_remap = ev.target.checked; } }),
        t('export.remap'),
        h('span', { class: 'small muted' }, t('export.remap.note'))),
      h('div', { class: 'small' }, t('export.gamedir'),
        h('input', {
          type: 'text', size: 60, value: S.run.game_dir || '', placeholder: (S.paths && S.paths.game_dir) || '',
          onchange: ev => { S.run.game_dir = ev.target.value.trim() || null; },
        }), ' ',
        h('button', { onclick: chooseGameDir }, t('export.browse'))),
      h('p', { class: 'small muted' }, t('export.gamedir.note'))),

    h('div', { class: 'panel' }, h('h3', null, t('export.h.dest')),
      h('p', null, h('code', null, S.exportDir || t('export.notchosen'))),
      isGameDir ? h('p', { class: 'small muted' }, t('export.dest.game') + (S.paths.gamesaves_exists ? '' : t('export.dest.create'))) : null,
      h('p', { class: 'small muted' }, t('export.dest.name')),
      h('div', { class: 'row' },
        h('button', { onclick: chooseExportDir }, t('export.choose')),
        S.paths && S.paths.gamesaves && !isGameDir ? h('button', { onclick: () => { S.exportDir = S.paths.gamesaves; paintExport(); } }, t('export.usegame')) : null)),

    h('div', { class: 'row actions' },
      h('button', { onclick: () => doExport(false), disabled: S.busy || !hasTemplate() }, t('export.btn.check')),
      h('button', { class: 'primary', onclick: () => doExport(true), disabled: S.busy || !hasTemplate() || !S.exportDir }, S.busy ? t('export.btn.working') : t('export.btn.export')),
      h('button', { onclick: saveRun }, t('review.saverun'))),

    resultView(S.exportResult)));
  if (S.exportResult) {
    const r = app.querySelector('.result');
    if (r) r.scrollIntoView({ block: 'nearest' });
  }
}

/// A manual template, or an automatic choice whose file was found.
function hasTemplate() {
  return !!(S.run.template || (S.plan && S.plan.choice && S.plan.choice.path));
}

async function chooseExportDir() {
  try {
    const d = await pickDir(t('pick.exportdir'));
    if (d) { S.exportDir = d; S.exportResult = null; paintExport(); }
  } catch (e) { banner('error', errText(e)); }
}

async function chooseGameDir() {
  try {
    const d = await pickDir(t('pick.gamedir'));
    if (d) { S.run.game_dir = d; S.exportResult = null; loadNames(); paintExport(); }
  } catch (e) { banner('error', errText(e)); }
}

async function doExport(write) {
  if (S.busy || !hasTemplate() || (write && !S.exportDir)) return;
  S.busy = true;
  banner('', write ? t('export.busy.write') : t('export.busy.check'));
  paintExport();
  try {
    const rep = write
      ? await invoke('export_save', { run: S.run, outDir: S.exportDir })
      : await invoke('check_export', { run: S.run });
    S.exportResult = { ok: rep };
    banner('ok', write ? t('export.done', { sav: rep.sav }) : t('export.checkok'));
  } catch (e) {
    const f = e && typeof e === 'object' && e.title ? e : { kind: 'io', title: t('export.failed'), hint: '', detail: String(e) };
    S.exportResult = { fail: f };
    banner('error', f.title);
  }
  S.busy = false;
  if (S.screen === 'export') paintExport();
}

// ---------------------------------------------------------------- back + keyboard
function back() {
  switch (S.screen) {
    case 'prologue': return go('welcome');
    case 'quest':
      if (S.step > 0) { S.step--; return quest(); }
      if (S.idx > 0) { S.idx--; S.step = 0; return quest(); }
      return go('prologue');
    case 'checkpoint': {
      const q = curQuest(); const e = entryOf(q.id);
      S.step = e && e.status === 'completed' ? (q.choices || []).length : 0;
      return go('quest');
    }
    case 'review': S.step = 0; return go(quests().length ? 'quest' : 'welcome');
    case 'export': return go('review');
    case 'side': return go('review');
    case 'safety': return go(S.safetyFrom || 'welcome');
    case 'editlist': S.mode = 'create'; return go('welcome');
    case 'editplan': return go('editlist');
    case 'editresult': return go('editplan');
  }
}

const KEYS = {
  welcome: { t: () => welcomeAction(0), d: () => welcomeAction(1), l: () => welcomeAction(2) },
  export: {
    c: () => doExport(false), e: () => doExport(true), f: chooseExportDir,
    n: () => { S.run.allow_name_remap = !S.run.allow_name_remap; paintExport(); },
  },
  review: { e: () => go('export') },
};

document.addEventListener('keydown', ev => {
  if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
  if (['SELECT', 'INPUT', 'TEXTAREA'].includes(ev.target.tagName)) return;
  if (!S.book) return;
  const k = ev.key;
  if (S.mode === 'edit') { if (k === 'Escape') { ev.preventDefault(); back(); } return; } // edit mode has no story keys
  const lower = k.length === 1 ? k.toLowerCase() : k;
  // A focused button keeps its own Enter/Space.
  if ((k === 'Enter' || k === ' ') && ev.target.tagName === 'BUTTON') return;
  if (k === 'ArrowDown') { ev.preventDefault(); moveSel(1); }
  else if (k === 'ArrowUp') { ev.preventDefault(); moveSel(-1); }
  else if (k === 'Enter' && S.list) { ev.preventDefault(); S.list.confirm(S.list.sel); }
  else if (k === 'Escape') { ev.preventDefault(); back(); }
  else if (KEYS[S.screen] && KEYS[S.screen][lower]) { ev.preventDefault(); KEYS[S.screen][lower](); }
  else if (lower === 's') stopHere();
  else if (lower === 'r' && S.screen !== 'welcome') go('review');
});

// ---------------------------------------------------------------- language
// The switch sits in the sidebar (#lang-slot). The choice is remembered by ui/i18n.js (localStorage).
function langPicker() {
  const sel = h('select', {
    class: 'lang-select', 'aria-label': t('lang.label'), title: t('lang.label'),
    onchange: async ev => {
      await I18N.set(ev.target.value);
      await loadNames();
      langPicker();
      imagesToggle();
      go(S.screen);
    },
  }, Object.entries(I18N.LANGS).map(([code, name]) => h('option', { value: code, selected: code === I18N.lang }, name)));
  document.getElementById('lang-slot').replaceChildren(sel);
}

// ---------------------------------------------------------------- boot
(async function boot() {
  await I18N.ready;
  try {
    S.book = await invoke('load_questbook');
    await loadBookTranslations(S.book);
    try { for (const l of await invoke('load_images')) S.images[l.quest] = l; } catch (_) { /* no illustrations */ }
    warmImages();
    const errs = S.book.diagnostics.filter(d => d.severity === 'error');
    if (errs.length) banner('error', tn('boot.qberr', errs.length));
  } catch (e) {
    banner('error', t('boot.fail', { e: errText(e) }));
    mount(h('div', { class: 'card' }, h('h2', null, t('boot.unavail')), h('p', { class: 'muted' }, errText(e))));
    return;
  }
  langPicker();
  imagesToggle();
  document.getElementById('safety-btn').addEventListener('click', () => { if (S.screen !== 'safety') S.safetyFrom = S.screen; go('safety'); });
  try { S.paths = await invoke('default_paths'); } catch (_) { S.paths = null; }
  await refreshPlan();
  go('welcome');
  loadNames().then(() => { if (S.screen !== 'export') SCREENS[S.screen](); });
})();
