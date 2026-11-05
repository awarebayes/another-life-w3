'use strict';
// Edit mode: open one of the player's own saves, see where it stands in the story and the answer it
// already holds for every decision, change what can be changed and write the result as a NEW save.
// Backend: list_saves, save_thumbnail, save_brief, edit_plan, edit_apply (src-tauri/src/main.rs,
// w3al_core::edit). The opened save is only read. Globals (h, mount, S, t, invoke, go, ...) come from app.js.

S.mode = 'create'; // 'create' (story -> new save) or 'edit' (change one of my saves)

function newEdit() {
  return {
    dir: '', saves: null, listError: null, sel: null,
    plan: null, loading: false, progress: '', error: null,
    changes: emptyChanges(), tab: 'all',
    outDir: '', busy: false, result: null, kitOpen: false,
    briefs: {}, thumbs: {},
    gameDir: null, templateDir: '',
  };
}
function emptyChanges() { return { answers: {}, level: null, crowns: null, gear: false, items: [], no_new_game_plus: false, skill_points: null }; }
S.edit = newEdit();
const E = S.edit; // never reassigned: newEdit() fields are copied in place by resetEdit()
function resetEdit() { Object.assign(E, newEdit(), { dir: E.dir, gameDir: E.gameDir, templateDir: E.templateDir, saves: E.saves, briefs: E.briefs, thumbs: E.thumbs }); }

if (TAURI && TAURI.event) {
  TAURI.event.listen('edit-progress', ev => { E.progress = String(ev.payload || ''); const p = document.getElementById('edit-progress'); if (p) p.textContent = E.progress; });
}

// ---------------------------------------------------------------- mode switch (Welcome and the save list)
function modeSwitch() {
  const on = S.mode === 'edit';
  return h('div', { class: 'tabs mode-switch', role: 'tablist', 'aria-label': t('edit.mode.aria') },
    h('button', { type: 'button', role: 'tab', class: 'tab' + (on ? '' : ' on'), 'aria-selected': String(!on), onclick: () => { S.mode = 'create'; go('welcome'); } }, t('edit.mode.create')),
    h('button', { type: 'button', role: 'tab', class: 'tab' + (on ? ' on' : ''), 'aria-selected': String(on), onclick: () => { S.mode = 'edit'; go('editlist'); } }, t('edit.mode.edit')));
}

// ---------------------------------------------------------------- helpers
// The folders the backend needs besides the save: the game install, the template folder and the donor folders
// (the Welcome screen's donors; the first window's graph effects copy from them).
const editFolders = () => ({
  gameDir: E.gameDir || S.run.game_dir || null,
  templateDirs: E.templateDir ? [E.templateDir] : (S.run.template_dirs || []),
  donors: S.run.donors || [],
});
const fmtDate = secs => { try { return new Date(secs * 1000).toLocaleString(I18N.lang === 'ru' ? 'ru-RU' : 'en-GB', { dateStyle: 'medium', timeStyle: 'short' }); } catch (_) { return String(secs); } };
const questById = id => (S.book ? quests().find(q => q.id === id) : null);
const choiceMeta = (qid, cid) => { const q = questById(qid); return q ? (q.choices || []).find(c => c.id === cid) : null; };
const optText = (qid, cid, oid) => { const c = choiceMeta(qid, cid); const o = c && c.options.find(o => o.id === oid); return o ? loc(o, 'text') : oid; };
const planChoice = (qid, cid) => { const q = E.plan && E.plan.quests.find(x => x.quest === qid); return q ? q.choices.find(c => c.id === cid) : null; };
const pendingOf = (qid, cid) => (E.changes.answers[qid] || {})[cid];

function changeCount() {
  const c = E.changes;
  return Object.values(c.answers).reduce((n, m) => n + Object.keys(m).length, 0)
    + (c.level != null ? 1 : 0) + (c.crowns != null ? 1 : 0) + (c.gear ? 1 : 0) + c.items.length + (c.no_new_game_plus ? 1 : 0) + (c.skill_points != null ? 1 : 0);
}
function setAnswer(qid, cid, oid) {
  const pc = planChoice(qid, cid);
  const m = E.changes.answers[qid] || (E.changes.answers[qid] = {});
  if (pc && pc.current === oid) delete m[cid]; else m[cid] = oid;
  if (!Object.keys(m).length) delete E.changes.answers[qid];
  E.result = null;
  editRefresh();
}
function undoAnswer(qid, cid) { setAnswer(qid, cid, (planChoice(qid, cid) || {}).current); }
function undoAllEdits() { E.changes = emptyChanges(); E.result = null; editRefresh(); }
function editRefresh() { if (S.screen === 'editplan') editplan(); else nav(); }

// ---------------------------------------------------------------- chrome (sidebar, top bar, ledger)
function editNav() {
  const on = (...s) => s.includes(S.screen);
  const sel = E.sel;
  const lvl = E.plan && E.plan.character ? E.plan.character.level : null;
  document.getElementById('sidecard').replaceChildren(
    h('span', { class: 'eyebrow' }, t('edit.side.eyebrow')),
    h('span', { class: 'name' + (sel ? '' : ' muted') }, sel ? sel.stem : t('edit.side.none')),
    h('span', { class: 'meta' }, lvl != null ? [h('span', { class: 'lvl' }, String(lvl)), ' ' + t('edit.side.level')] : t('edit.side.hint')));
  const item = (label, o) => h('button', { class: 'nav-item' + (o.on ? ' on' : ''), disabled: o.disabled, onclick: o.go, type: 'button' },
    h('span', { class: 'label' }, label), o.count != null ? h('span', { class: 'count' }, String(o.count)) : null);
  document.getElementById('nav').replaceChildren(
    h('div', { class: 'nav-sec' }, h('div', { class: 'nav-label' }, t('edit.nav.label')),
      item(t('edit.nav.saves'), { on: on('editlist'), go: () => go('editlist') }),
      item(t('edit.nav.decisions'), { on: on('editplan'), disabled: !sel, count: changeCount() || null, go: () => go('editplan') }),
      item(t('edit.nav.result'), { on: on('editresult'), disabled: !E.result, go: () => go('editresult') })),
    h('div', { class: 'nav-sec' }, h('div', { class: 'nav-label' }, t('edit.nav.other')),
      item(t('edit.nav.create'), { go: () => { S.mode = 'create'; go('welcome'); } })));

  const n = changeCount();
  document.getElementById('ctx').replaceChildren(
    h('span', { class: 'ctx-name' }, sel ? sel.stem : t('edit.top.none')),
    h('span', { class: 'ctx-meta' }, lvl != null ? t('top.level', { n: lvl }) : ''));
  document.getElementById('topactions').replaceChildren(
    h('span', { class: 'status-pill' + (n ? ' dirty' : '') }, n ? tn('edit.top.unsaved', n) : t('top.nochanges')),
    E.plan ? h('button', { class: 'ghost', onclick: undoAllEdits, disabled: !n }, t('top.undoall')) : null,
    E.plan ? h('button', { class: 'primary', onclick: editApply, disabled: !n || E.busy }, t('edit.save')) : null,
    h('button', { onclick: closeApp }, t('top.close')));

  document.getElementById('body').classList.toggle('no-ledger', !on('editplan', 'editresult'));
  const list = [];
  for (const [qid, m] of Object.entries(E.changes.answers)) {
    for (const [cid, oid] of Object.entries(m)) {
      const pc = planChoice(qid, cid) || {};
      const q = questById(qid);
      list.push(h('li', null,
        h('span', { class: 'q' }, q ? qname(q) : qid),
        h('span', { class: 'a' }, pc.current ? t('edit.ledger.from', { from: optText(qid, cid, pc.current), to: optText(qid, cid, oid) }) : optText(qid, cid, oid)),
        h('div', { class: 'row' }, h('span', { class: 'st' }, pc.mode === 'synth' ? t('edit.ledger.synth') : pc.mode === 'late' ? t('edit.ledger.late') : t('edit.ledger.rewrite')),
          h('button', { class: 'link', onclick: () => undoAnswer(qid, cid) }, t('ledger.undo')))));
    }
  }
  const c = E.changes;
  const kitLine = (key, text, undo) => list.push(h('li', null, h('span', { class: 'q' }, t(key)), h('span', { class: 'a' }, text),
    h('div', { class: 'row' }, h('span', { class: 'st' }, t('edit.ledger.char')), h('button', { class: 'link', onclick: undo }, t('ledger.undo')))));
  if (c.level != null) kitLine('edit.ledger.level', t('edit.ledger.level.to', { n: c.level }), () => { c.level = null; editRefresh(); });
  if (c.crowns != null) kitLine('edit.ledger.crowns', t('edit.ledger.crowns.to', { n: c.crowns }), () => { c.crowns = null; editRefresh(); });
  if (c.gear) kitLine('edit.ledger.gear', t('edit.ledger.gear.to'), () => { c.gear = false; editRefresh(); });
  for (const name of c.items) kitLine('edit.ledger.item', iname(name), () => { c.items = c.items.filter(x => x !== name); editRefresh(); });
  if (c.skill_points != null) kitLine('edit.ledger.skill', t('edit.ledger.skill.to', { n: c.skill_points }), () => { c.skill_points = null; editRefresh(); });
  if (c.no_new_game_plus) kitLine('edit.ledger.ngplus', t('edit.ledger.ngplus.to'), () => { c.no_new_game_plus = false; editRefresh(); });
  const dest = E.outDir || E.dir;
  document.getElementById('ledger').replaceChildren(
    h('div', { class: 'ledger-head' }, h('h3', null, t('ledger.title')), h('div', { class: 'sub' }, tn('edit.ledger.count', n))),
    list.length ? h('ul', { class: 'ledger-list' }, list) : h('div', { class: 'ledger-empty' }, t('edit.ledger.empty')),
    h('div', { class: 'ledger-foot' },
      h('p', { style: 'margin:0 0 8px' }, t('edit.ledger.dest', { dir: dest || t('export.notchosen') })),
      h('div', { class: 'row' },
        h('button', { class: 'primary', onclick: editApply, disabled: !n || E.busy || !E.plan }, E.busy ? t('export.btn.working') : t('edit.save')),
        h('button', { class: 'link', onclick: chooseOutDir }, t('edit.ledger.other')))));
}

async function chooseOutDir() {
  try { const d = await pickDir(t('pick.exportdir')); if (d) { E.outDir = d; editRefresh(); } } catch (e) { banner('error', errText(e)); }
}

// ---------------------------------------------------------------- 1. the save list
async function loadSaveList() {
  E.saves = null; E.listError = null;
  editlist();
  try { E.saves = await invoke('list_saves', { dir: E.dir }); } catch (e) { E.saves = []; E.listError = errText(e); }
  if (S.screen === 'editlist') { editlist(); fillCards(); }
}
async function pickSavesDir() {
  try { const d = await pickDir(t('edit.list.choose')); if (d) { E.dir = d; E.outDir = ''; loadSaveList(); } } catch (e) { banner('error', errText(e)); }
}
// Thumbnails and level / crowns are read one by one after the grid is shown.
async function fillCards() {
  if (E.filling) return;
  E.filling = true;
  try { await fillCardsRun(); } finally { E.filling = false; }
}
async function fillCardsRun() {
  for (const s of E.saves || []) {
    if (S.screen !== 'editlist') return;
    if (s.thumbnail && !(s.path in E.thumbs)) {
      try { E.thumbs[s.path] = await invoke('save_thumbnail', { path: s.thumbnail }); } catch (_) { E.thumbs[s.path] = null; }
      const img = document.querySelector(`[data-thumb="${CSS.escape(s.path)}"]`);
      if (img && E.thumbs[s.path]) img.replaceChildren(h('img', { src: E.thumbs[s.path], alt: '' }));
    }
  }
  for (const s of E.saves || []) {
    if (S.screen !== 'editlist') return;
    if (!(s.path in E.briefs)) {
      try { E.briefs[s.path] = await invoke('save_brief', { path: s.path }); } catch (_) { E.briefs[s.path] = {}; }
      const el = document.querySelector(`[data-brief="${CSS.escape(s.path)}"]`);
      if (el) el.replaceChildren(...briefBits(E.briefs[s.path]));
    }
  }
}
function briefBits(b) {
  if (!b || (b.level == null && !b.world)) return [h('span', { class: 'muted' }, t('edit.card.unread'))];
  const w = b.world ? worldLabel(b.world) : null;
  return [b.level != null ? h('span', null, h('span', { class: 'lvl' }, String(b.level)), ' ' + t('edit.card.level')) : null,
    b.crowns != null ? h('span', null, t('edit.card.crowns', { n: b.crowns })) : null,
    w ? h('span', { class: 'muted' }, w) : null];
}

function editlist() {
  hints(['Esc', 'hint.back']);
  if (!E.dir && S.paths && S.paths.gamesaves) E.dir = S.paths.gamesaves;
  if (E.saves == null && E.dir && !E.listError && !E.listing) { E.listing = true; loadSaveList().finally(() => { E.listing = false; }); }
  const cards = (E.saves || []).map(s => h('button', { type: 'button', class: 'save-card' + (E.sel && E.sel.path === s.path ? ' on' : ''), onclick: () => editOpen(s) },
    h('span', { class: 'thumb', 'data-thumb': s.path }, E.thumbs[s.path] ? h('img', { src: E.thumbs[s.path], alt: '' }) : h('span', { class: 'nothumb' }, s.thumbnail ? '' : t('edit.card.nothumb'))),
    h('span', { class: 'save-meta' },
      h('span', { class: 'save-stem' }, s.stem),
      h('span', { class: 'save-when' }, fmtDate(s.modified), ' · ', h('span', { class: 'kind' }, t('edit.kind.' + s.kind)), ' · ', t('edit.card.size', { mb: (s.size / 1048576).toFixed(1) })),
      h('span', { class: 'save-brief', 'data-brief': s.path }, E.briefs[s.path] ? briefBits(E.briefs[s.path]) : ''))));
  mount(h('div', { class: 'page' },
    h('div', { class: 'page-head' }, h('h1', null, t('edit.list.h1')), h('span', { class: 'pill safe' }, t('edit.list.pill'))),
    h('p', { class: 'lede' }, t('edit.list.lede')),
    modeSwitch(),
    h('div', { class: 'panel' },
      h('h3', null, t('edit.list.folder')),
      h('p', null, h('code', null, E.dir || t('export.notchosen'))),
      h('div', { class: 'row' },
        h('button', { onclick: pickSavesDir }, t('edit.list.choose')),
        S.paths && S.paths.gamesaves && E.dir !== S.paths.gamesaves ? h('button', { onclick: () => { E.dir = S.paths.gamesaves; E.outDir = ''; loadSaveList(); } }, t('edit.list.usegame')) : null,
        h('button', { class: 'ghost', onclick: loadSaveList, disabled: !E.dir }, t('edit.list.refresh')))),
    h('details', { class: 'panel' },
      h('summary', null, h('span', { class: 'eyebrow' }, t('edit.list.folders'))),
      h('p', { class: 'small muted', style: 'margin-top:12px' }, t('edit.list.folders.note')),
      h('div', { class: 'small' }, t('export.gamedir'),
        h('input', { type: 'text', size: 56, value: E.gameDir || '', placeholder: (S.paths && S.paths.game_dir) || '', onchange: ev => { E.gameDir = ev.target.value.trim() || null; E.plan = null; } }),
        ' ', h('button', { onclick: async () => { const d = await pickDir(t('pick.gamedir')); if (d) { E.gameDir = d; E.plan = null; editlist(); } } }, t('export.browse'))),
      h('div', { class: 'small', style: 'margin-top:8px' }, t('edit.list.tpldir'),
        h('input', { type: 'text', size: 56, value: E.templateDir || '', placeholder: t('edit.list.tpldir.ph'), onchange: ev => { E.templateDir = ev.target.value.trim(); E.plan = null; } }),
        ' ', h('button', { onclick: async () => { const d = await pickDir(t('pick.tpldir')); if (d) { E.templateDir = d; E.plan = null; editlist(); } } }, t('export.browse')))),
    E.listError ? h('div', { class: 'err-box' }, E.listError) : null,
    E.saves == null ? h('p', { class: 'muted' }, t('edit.list.loading'))
      : cards.length ? h('div', { class: 'save-grid' }, cards)
      : h('p', { class: 'muted' }, t('edit.list.none'))));
  if (E.saves) fillCards();
}

// ---------------------------------------------------------------- 2. the decisions of one save
async function editOpen(entry) {
  resetEdit();
  E.sel = entry; E.loading = true; E.progress = ''; E.plan = null; E.error = null;
  go('editplan');
  try {
    E.plan = await invoke('edit_plan', { savePath: entry.path, ...editFolders() });
  } catch (e) { E.error = e && typeof e === 'object' && e.title ? e : { title: t('edit.plan.failed'), hint: '', detail: errText(e) }; }
  E.loading = false;
  if (S.screen === 'editplan') editplan(); else nav();
}

const STATUS_SEAL = { done: 'decided', active: 'default', not_reached: 'notreached', unknown: 'notreached', untracked: 'notreached' };

function questRow(pq) {
  const q = questById(pq.quest);
  if (!q) return null;
  const choices = pq.choices;
  const blocks = choices.map(pc => {
    const meta = choiceMeta(q.id, pc.id);
    if (!meta) return null;
    const pend = pendingOf(q.id, pc.id);
    const eff = pend || pc.current;
    const synth = pc.mode === 'synth' || pc.mode === 'late';
    const alts = pc.options.filter(o => o.allowed && o.id !== eff && (pc.mode === 'rewrite' || synth))
      .map(o => h('button', { type: 'button', onclick: () => setAnswer(q.id, pc.id, o.id) }, optText(q.id, pc.id, o.id) + (synth && o.id === meta.default ? ' · ' + t('edit.default') : '')));
    let outcome;
    if (eff) {
      outcome = h('div', { class: 'outcome' + (pend ? ' changed' : '') }, optText(q.id, pc.id, eff),
        pend ? h('span', { class: 'was' }, pc.current ? ' ' + t('edit.answer.changed', { old: optText(q.id, pc.id, pc.current) }) : '') : null,
        pend ? h('button', { class: 'link small', onclick: () => undoAnswer(q.id, pc.id) }, t('ledger.undo')) : null);
    } else {
      outcome = h('div', { class: 'outcome dim' }, pq.status === 'done' ? t('edit.answer.unknown') : t('edit.answer.undecided'));
    }
    const blocked = pc.mode === 'readonly';
    return h('div', { class: 'decision' + (blocked ? ' readonly' : '') },
      h('div', { class: 'dprompt' }, loc(meta, 'prompt')),
      outcome,
      alts.length ? h('div', { class: 'alts' }, alts) : null,
      blocked && pc.reason ? h('div', { class: 'why small' }, t('edit.cannot', { why: pc.reason })) : null,
      pc.mode === 'late' ? h('div', { class: 'why small' }, t('edit.late.hint')) : null,
      !blocked && pc.source === 'registry' ? h('div', { class: 'why small' }, t('edit.fromregistry')) : null);
  });
  const note = !choices.length ? (pq.status === 'done' ? t('edit.nochoices.done') : pq.status === 'active' ? t('edit.nochoices.active') : (pq.note || t('edit.st.' + pq.status))) : null;
  return h('li', { class: 'tl' }, h('span', { class: 'seal ' + STATUS_SEAL[pq.status], title: t('edit.st.' + pq.status) }),
    h('div', null,
      h('div', { class: 'tl-head' },
        h('span', { class: 'tl-title' }, qname(q)),
        h('span', { class: 'tl-where' }, t('review.where', { act: ROMAN[q.act] || q.act, region: regionName(q.region), n: q.level })),
        h('span', { class: 'st st-' + pq.status }, t('edit.st.' + pq.status))),
      note ? h('div', { class: 'outcome dim' }, note) : null,
      pq.status === 'active' && choices.length ? h('p', { class: 'small muted', style: 'margin:0 0 6px' }, t('edit.active.hint')) : null,
      pq.status === 'unknown' && pq.note && choices.length ? h('p', { class: 'small muted', style: 'margin:0 0 6px' }, pq.note) : null,
      ...blocks));
}

function editplan() {
  hints(['Esc', 'hint.back']);
  const head = h('div', { class: 'page-head' }, h('h1', null, t('edit.plan.h1')), h('span', { class: 'pill caution' }, t('edit.plan.pill')));
  if (E.loading) {
    return mount(h('div', { class: 'page' }, head,
      h('p', { class: 'lede' }, t('edit.plan.loading')),
      h('div', { class: 'progress indet' }, h('i')),
      h('p', { class: 'small muted', id: 'edit-progress' }, E.progress || ''),
      h('p', { class: 'small muted' }, t('edit.plan.loading.sub'))));
  }
  if (E.error) {
    const f = E.error;
    return mount(h('div', { class: 'page' }, head,
      h('div', { class: 'result fail' }, h('h2', null, f.title), f.hint ? h('p', null, f.hint) : null, f.detail ? h('pre', { class: 'out' }, f.detail) : null),
      h('div', { class: 'row actions' }, h('button', { onclick: () => go('editlist') }, t('edit.result.other')), h('button', { onclick: () => editOpen(E.sel) }, t('edit.retry')))));
  }
  const plan = E.plan;
  if (!plan) return mount(h('div', { class: 'page' }, head, h('button', { onclick: () => go('editlist') }, t('edit.result.other'))));
  // A repaint after a click keeps the scroll position and whether the character card is open.
  const again = !!document.querySelector('.timeline');
  const keepTop = again ? app.scrollTop : 0;
  const kitEl = document.getElementById('editkit');
  if (kitEl) E.kitOpen = kitEl.open;
  const count = s => plan.quests.filter(q => q.status === s).length;
  const changeable = pq => pq.choices.some(c => c.mode === 'rewrite' || c.mode === 'synth' || c.mode === 'late');
  const tabs = [['all', t('review.tab.all'), plan.quests.length], ['changeable', t('edit.tab.changeable'), plan.quests.filter(changeable).length],
    ['readonly', t('edit.tab.readonly'), plan.quests.filter(q => !changeable(q)).length]];
  const shown = plan.quests.filter(q => E.tab === 'all' || (E.tab === 'changeable') === changeable(q));
  const ch = plan.character;
  mount(h('div', { class: 'page' }, head,
    h('p', { class: 'lede' }, t('edit.plan.lede')),
    h('div', { class: 'plan-grid' },
      h('div', { class: 'tile' }, h('span', { class: 'eyebrow' }, t('edit.tile.story')),
        h('span', { class: 'big' }, String(count('done')), h('span', { class: 'of' }, ` / ${plan.quests.length}`)),
        h('span', { class: 'small muted' }, t('edit.tile.story.sub', { active: count('active'), ahead: count('not_reached'), unclear: count('unknown') + count('untracked') }))),
      h('div', { class: 'tile' }, h('span', { class: 'eyebrow' }, t('edit.tile.position')),
        h('span', { class: 'big' }, plan.position || '-'),
        h('span', { class: 'small muted' }, plan.is_template ? t('edit.tile.istemplate') : t('edit.tile.position.sub'))),
      h('div', { class: 'tile' }, h('span', { class: 'eyebrow' }, t('edit.tile.char')),
        h('span', { class: 'big' }, ch ? String(ch.level) : '-'),
        h('span', { class: 'small muted' }, ch ? t('edit.tile.char.sub', { crowns: ch.crowns == null ? '-' : ch.crowns, items: ch.items }) : t('edit.kit.err', { e: plan.character_error || '' })))),
    h('div', { style: 'height:16px' }),
    ...plan.notes.map(n => h('p', { class: 'small muted' }, n)),
    editKitCard(plan),
    h('div', { class: 'row spread', style: 'margin:8px 0 16px' },
      h('div', { class: 'tabs', style: 'margin:0' }, tabs.map(([k, label, n]) => h('button', { type: 'button', class: 'tab' + (k === E.tab ? ' on' : ''), onclick: () => { E.tab = k; editplan(); } }, label, h('span', { class: 'n' }, String(n))))),
      h('button', { class: 'primary', onclick: editApply, disabled: !changeCount() || E.busy }, t('edit.save'))),
    shown.length ? h('ol', { class: 'timeline' }, shown.map(questRow)) : h('p', { class: 'muted' }, t('review.empty'))));
  app.scrollTop = keepTop;
}

// Level, crowns, the recommended kit and single items of the kit table.
function editKitCard(plan) {
  const ch = plan.character;
  if (!ch) return h('div', { class: 'warn-box' }, t('edit.kit.err', { e: plan.character_error || '' }));
  const c = E.changes;
  const target = c.level != null ? c.level : ch.level;
  const band = ch.kit.filter(k => k.from_level <= target).pop() || ch.kit[0];
  const set = (k, v) => { c[k] = v; E.result = null; editplan(); };
  const stepper = (key, min, max, cur, step) => {
    const input = h('input', { type: 'number', min, max, value: c[key] == null ? '' : c[key], placeholder: String(cur),
      onchange: ev => set(key, ev.target.value === '' || Number.isNaN(parseInt(ev.target.value, 10)) ? null : Math.max(min, Math.min(max, parseInt(ev.target.value, 10)))) });
    const bump = d => () => set(key, Math.max(min, Math.min(max, (c[key] == null ? cur : c[key]) + d)));
    return h('span', { class: 'stepper' }, h('button', { type: 'button', 'aria-label': t('kit.less'), onclick: bump(-step) }, '−'), input,
      h('button', { type: 'button', 'aria-label': t('kit.more'), onclick: bump(step) }, '+'));
  };
  const addable = band.items.filter(i => !c.items.includes(i.name));
  const sel = h('select', { 'aria-label': t('edit.kit.item') }, h('option', { value: '' }, t('edit.kit.item.pick')),
    addable.map(i => h('option', { value: i.name }, iname(i.name) + (i.carried != null ? ' · ' + t('edit.kit.carried', { n: i.carried }) : ''))));
  return h('details', { class: 'panel start-kit', id: 'editkit', open: !!(E.kitOpen || c.level != null || c.crowns != null || c.gear || c.items.length || c.no_new_game_plus || c.skill_points != null) },
    h('summary', null, h('span', { class: 'eyebrow' }, t('edit.kit.title'))),
    h('p', { class: 'small muted', style: 'margin-top:12px' }, t('edit.kit.intro')),
    h('div', { class: 'kit-grid' },
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.level')),
        stepper('level', 1, ch.max_level || 50, ch.level, 1),
        h('span', { class: 'was' }, t('edit.kit.now', { n: ch.level }))),
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.crowns')),
        stepper('crowns', 0, 999999, ch.crowns || 0, 100),
        h('span', { class: 'was' }, ch.crowns == null ? t('edit.kit.nocrowns') : t('edit.kit.now.crowns', { n: ch.crowns }))),
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.skill')),
        stepper('skill_points', ch.skill_used || 0, 300, (ch.skill_free || 0) + (ch.skill_used || 0), 1),
        h('span', { class: 'was' }, t('edit.kit.skill.now', { n: (ch.skill_free || 0) + (ch.skill_used || 0), used: ch.skill_used || 0, lvl: target, grant: Math.max(0, target - 1) }))),
      h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.supplies')),
        h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: c.gear, onchange: ev => set('gear', ev.target.checked) }), t('edit.kit.recommended')),
        h('span', { class: 'was' }, t('edit.kit.recommended.note', { n: band.items.length }))),
      ch.new_game_plus ? h('div', { class: 'field' }, h('span', { class: 'lab' }, t('kit.ngplus')),
        h('label', { class: 'check' }, h('input', { type: 'checkbox', checked: c.no_new_game_plus, onchange: ev => set('no_new_game_plus', ev.target.checked) }), t('kit.ngplus.off')),
        h('span', { class: 'was' }, t('kit.ngplus.note'))) : null),
    h('div', { class: 'field', style: 'margin-top:16px' }, h('span', { class: 'lab' }, t('edit.kit.item')),
      h('div', { class: 'row' }, sel, h('button', { type: 'button', onclick: () => { if (sel.value) { c.items.push(sel.value); E.result = null; editplan(); } } }, t('edit.kit.add'))),
      c.items.length ? h('div', { class: 'chips' }, c.items.map(n => h('span', { class: 'chip' }, iname(n), h('button', { type: 'button', class: 'link', 'aria-label': t('ledger.undo'), onclick: () => { c.items = c.items.filter(x => x !== n); editplan(); } }, '×')))) : null));
}

// ---------------------------------------------------------------- 3. write the new save
async function editApply() {
  if (E.busy || !changeCount() || !E.plan) return;
  E.busy = true;
  banner('', t('edit.busy'));
  nav();
  const c = E.changes;
  try {
    const rep = await invoke('edit_apply', {
      savePath: E.sel.path,
      changes: { answers: c.answers, level: c.level, crowns: c.crowns, gear: c.gear, items: c.items },
      outDir: E.outDir || E.dir,
      folders: editFolders(),
    });
    E.result = { ok: rep };
    banner('ok', t('edit.done', { sav: rep.sav }));
  } catch (e) {
    const f = e && typeof e === 'object' && e.title ? e : { title: t('export.failed'), hint: '', detail: String(e) };
    E.result = { fail: f };
    banner('error', f.title);
  }
  E.busy = false;
  go('editresult');
}

function editresult() {
  hints(['Esc', 'hint.back']);
  const r = E.result;
  if (!r) return go('editplan');
  const body = r.fail
    ? h('div', { class: 'result fail' }, h('h2', null, r.fail.title), r.fail.hint ? h('p', null, r.fail.hint) : null, r.fail.detail ? h('pre', { class: 'out' }, r.fail.detail) : null)
    : h('div', { class: 'result ok' },
      h('h2', null, t('edit.result.untouched')),
      r.ok.rewritten.length ? h('p', null, t('edit.result.rewritten', { list: r.ok.rewritten.map(questTitle).join(', ') })) : null,
      r.ok.applied.length ? h('p', null, t('edit.result.applied', { list: r.ok.applied.map(questTitle).join(', ') })) : null,
      r.ok.notes.length ? h('ul', { class: 'plain small' }, r.ok.notes.map(n => h('li', null, n))) : null,
      h('p', { class: 'verified' }, r.ok.verified),
      h('p', null, t('res.written')),
      h('ul', { class: 'plain small' }, [r.ok.sav, ...r.ok.sidecars].map(f => h('li', null, h('code', null, f)))),
      h('p', { class: 'small muted' }, t('res.ingame')));
  mount(h('div', { class: 'page' },
    h('div', { class: 'page-head' }, h('h1', null, r.fail ? t('edit.result.h.fail') : t('edit.result.h')), h('span', { class: 'pill safe' }, t('edit.list.pill'))),
    body,
    h('div', { class: 'row actions' },
      h('button', { onclick: () => go('editplan') }, t('edit.result.again')),
      h('button', { onclick: () => { E.sel = null; E.plan = null; go('editlist'); } }, t('edit.result.other')))));
}

// ---------------------------------------------------------------- browser preview (no Tauri): invented data
async function editMock(cmd, args) {
  const book = () => S.book.quests;
  if (cmd === 'list_saves') {
    const now = Math.floor(Date.now() / 1000);
    return [
      ['ManualSave_4dcef_7ea51000_400b253', 'manual', now - 3600], ['QuickSave_52586_7ea4c400_1030937', 'quick', now - 86400],
      ['ManualSave_4e4ce_7ea51000_3fb3aa7', 'manual', now - 3 * 86400], ['AutoSave_6255c_7ea51000_8f3325', 'auto', now - 9 * 86400],
      ['CheckPoint_568ef_7ea51000_3ad61fa', 'checkpoint', now - 20 * 86400], ['ManualSave_10028e_7ea51000_3f6825b', 'manual', now - 40 * 86400],
    ].map(([stem, kind, modified], i) => ({ stem, path: 'C:\\saves\\' + stem + '.sav', kind, modified, size: 1500000 + i * 210000, thumbnail: i === 3 ? null : 'C:\\saves\\' + stem + '.png' }));
  }
  if (cmd === 'save_thumbnail') {
    const hue = [...args.path].reduce((a, c) => a + c.charCodeAt(0), 0) % 360;
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="228" height="128"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue},12%,22%)"/><stop offset="1" stop-color="hsl(${(hue + 40) % 360},14%,12%)"/></linearGradient></defs><rect width="228" height="128" fill="url(#g)"/><path d="M0 100 L50 70 L90 92 L140 56 L190 88 L228 66 V128 H0Z" fill="rgba(0,0,0,.35)"/></svg>`;
    return 'data:image/svg+xml;base64,' + btoa(svg);
  }
  if (cmd === 'save_brief') return { level: 28 + (args.path.length % 9), crowns: 4210 + args.path.length * 37, world: 'levels\\novigrad\\novigrad.w2w', error: null };
  if (cmd === 'edit_plan') {
    await new Promise(r => setTimeout(r, 900));
    const qs = book();
    const cut = Math.max(2, Math.floor(qs.length * 0.6));
    const quests = qs.map((q, i) => {
      const status = i < cut ? (i % 9 === 4 ? 'unknown' : 'done') : i === cut ? 'active' : 'not_reached';
      const choices = (q.choices || []).map((c, k) => {
        const cur = c.options[(i + k) % c.options.length].id;
        if (status === 'done') {
          const locked = i % 3 === 1;
          return { id: c.id, current: cur, source: i % 5 === 0 ? 'registry' : 'facts', mode: locked ? 'readonly' : 'rewrite',
            options: c.options.map(o => ({ id: o.id, allowed: !locked, why: null })),
            reason: locked ? 'the battle of Kaer Morhen (q403, pack 47) already played with these allies (preview text)' : null };
        }
        if (status === 'active') return { id: c.id, current: null, source: 'none', mode: 'synth', options: c.options.map(o => ({ id: o.id, allowed: true, why: null })), reason: null };
        if (status === 'not_reached' && i === cut + 1) return { id: c.id, current: null, source: 'none', mode: 'late', options: c.options.map(o => ({ id: o.id, allowed: true, why: null })), reason: null };
        return { id: c.id, current: null, source: 'none', mode: 'readonly', options: c.options.map(o => ({ id: o.id, allowed: false, why: null })), reason: status === 'unknown' ? 'mixed: the best answer still differs in 4 of 9 deciding facts' : 'this part of the story is not reached in the save' };
      });
      return { quest: q.id, window: 'w' + (1 + (i >> 1)), status, note: status === 'unknown' ? 'mixed: the best answer still differs in 4 of 9 deciding facts' : '', choices };
    });
    const bands = [1, 11, 15, 18, 26, 30, 34].map(from => ({ from_level: from, items: [
      { name: from < 11 ? 'Dol Blathanna longsword' : 'Gryphon School steel sword', quantity: 1, form: 'gear', carried: null },
      { name: 'Swallow 1', quantity: 1, form: 'charged', carried: 2 }, { name: 'Cat 1', quantity: 1, form: 'charged', carried: null },
      { name: 'Beast Oil 1', quantity: 1, form: 'charged', carried: null }, { name: 'Bread', quantity: 5, form: 'stack', carried: 3 }, { name: 'Blunt Bolt', quantity: 20, form: 'stack', carried: 12 }] }));
    return { stem: args.savePath.split(/[\\/]/).pop().replace(/\.sav$/i, ''), position: 'w9_skellige_act3', is_template: null,
      windows: [], quests, character: { level: 31, max_level: 50, crowns: 5230, items: 641, new_game_plus: true, skill_free: 30, skill_used: 2, kit: bands }, character_error: null,
      notes: ['Browser preview: this plan is invented from the questbook to show the screens. Nothing was read.'] };
  }
  if (cmd === 'edit_apply') {
    await new Promise(r => setTimeout(r, 700));
    const stem = 'ManualSave_4dcef_7ea51000_41a9c02';
    return { sav: args.outDir + '\\' + stem + '.sav (preview)', sidecars: [args.outDir + '\\' + stem + '.png (preview)', args.outDir + '\\' + stem + '.json (preview)'],
      notes: ['preview: nothing was written'], rewritten: Object.keys(args.changes.answers).slice(0, 2), applied: [],
      verified: 'VERIFIED (preview): the browser preview writes nothing.' };
  }
  return undefined;
}
