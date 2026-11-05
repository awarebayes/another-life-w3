'use strict';
// Side quests page (questbook/side, crate::side): quests without a consequence the player chooses (side quests,
// contracts, treasure and scavenger hunts, Gwent, races, fist fights, main quests between decisions). Checked =
// completed: export finishes it in the save with the auto-complete walk (w3graph::complete); unchecked = skipped.
// Quests the run's template already has done are shown checked and disabled. Loaded after app.js.

const SIDE_KINDS = ['main', 'side', 'contract', 'treasure', 'scavenger', 'gwent', 'race', 'fistfight'];
// Official title from the player's install (backend `game_names`, by `title_id`), else the internal graph path.
const sideTitle = q => (S.names.side && S.names.side[q.id]) || q.fallback_title.split('/').pop();
const sideList = () => (S.book && S.book.side) || [];
const sideCount = () => Object.values(S.run.side_quests || {}).filter(s => s === 'completed').length;

async function sideStates() {
  const key = JSON.stringify([S.run.template, S.run.template_dirs, S.run.stop_after, S.run.quests]);
  if (S.sideStates && S.sideStatesKey === key) return S.sideStates;
  try { S.sideStates = await invoke('side_quest_states', { run: S.run }); } catch (e) { S.sideStates = { template: null, template_id: null, done: [], error: errText(e) }; }
  S.sideStatesKey = key;
  return S.sideStates;
}

function setSide(ids, on) {
  const m = S.run.side_quests || (S.run.side_quests = {});
  for (const id of ids) {
    if (on) m[id] = 'completed'; else delete m[id];
  }
  S.exportResult = null;
  sideScreen();
}

async function sideScreen() {
  hints(['Esc', 'hint.backstory'], ['R', 'hint.review']);
  const all = sideList();
  const st = await sideStates();
  if (S.screen !== 'side') return;
  const done = new Set(st.done || []);
  const chosen = S.run.side_quests || {};
  const acts = [...new Set(all.map(q => q.act))].sort((a, b) => a - b);
  if (!acts.includes(S.sideAct)) S.sideAct = acts[0];
  const inAct = all.filter(q => q.act === S.sideAct);
  const regions = [...new Set(inAct.map(q => q.region))];
  if (!regions.includes(S.sideRegion)) S.sideRegion = regions[0];
  const shown = inAct.filter(q => q.region === S.sideRegion);
  const open = shown.filter(q => !done.has(q.id));

  const row = q => {
    const isDone = done.has(q.id);
    const trial = st.template_id && q.templates ? q.templates[st.template_id] : null;
    const warn = !isDone && trial && trial !== 'ok' && trial !== 'done' ? t('sidepage.unavailable', { why: trial }) : null;
    return h('li', { title: q.fallback_title }, h('label', { class: 'check' + (isDone ? ' muted' : '') },
      h('input', { type: 'checkbox', checked: isDone || chosen[q.id] === 'completed', disabled: isDone, onchange: ev => setSide([q.id], ev.target.checked) }),
      h('span', null, sideTitle(q),
        isDone ? h('span', { class: 'note' }, ' ' + t('sidepage.done')) : null,
        warn ? h('span', { class: 'note unfinished' }, ' ' + warn) : null)));
  };
  const groups = SIDE_KINDS.map(k => [k, shown.filter(q => q.kind === k)]).filter(([, qs]) => qs.length).map(([k, qs]) =>
    h('section', { class: 'sq-group' },
      h('h3', null, t('sidepage.kind.' + k), h('span', { class: 'n' }, ' ' + qs.length)),
      h('ul', { class: 'sq-list plain' }, qs.map(row))));

  mount(h('div', { class: 'page' },
    h('div', { class: 'page-head' }, h('h1', null, t('sidepage.h1')), h('span', { class: 'pill' }, tn('sidepage.count', sideCount()))),
    h('p', { class: 'lede' }, t('sidepage.lede')),
    st.error ? h('div', { class: 'callout' }, t('sidepage.notemplate', { why: st.error }))
      : h('p', { class: 'small muted' }, t('sidepage.template', { name: st.template_id || st.template || '-' })),
    all.length ? [
      h('div', { class: 'tabs' }, acts.map(a => h('button', {
        type: 'button', class: 'tab' + (a === S.sideAct ? ' on' : ''), onclick: () => { S.sideAct = a; sideScreen(); },
      }, t('sidepage.act', { n: ROMAN[a] || a }), h('span', { class: 'n' }, String(all.filter(q => q.act === a).length))))),
      h('div', { class: 'row spread', style: 'margin-bottom:12px' },
        h('select', { 'aria-label': t('review.region'), onchange: ev => { S.sideRegion = ev.target.value; sideScreen(); } },
          regions.map(r => h('option', { value: r, selected: r === S.sideRegion }, regionName(r)))),
        h('div', { class: 'row' },
          h('button', { onclick: () => setSide(open.map(q => q.id), true), disabled: !open.length }, t('sidepage.markall')),
          h('button', { class: 'ghost', onclick: () => setSide(open.map(q => q.id), false), disabled: !open.some(q => chosen[q.id]) }, t('sidepage.clear')))),
      ...groups,
    ] : h('p', { class: 'muted' }, t('sidepage.empty'))));
}
