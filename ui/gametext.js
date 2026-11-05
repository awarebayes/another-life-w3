'use strict';
// The game's own quest texts (descriptions, objective titles, character names) in the player's language.
// Backend command `game_texts` reads them at run time from the player's install; nothing is stored in
// the repository or on disk, and without an install every lookup returns null / [] so the UI keeps its
// own narration. Plain data only: app.js decides how to render.
//
//   await GameText.load(lang, gameDir)   (re)load; call on boot and when the language or game folder changes
//   GameText.found                       true when an install with the language's strings was read
//   GameText.guid(q)                     journal quest GUID of a questbook quest (q.journal_guid, a 32-hex
//                                        q.journal, else matched by official English title), or null
//   GameText.title(q)                    official title or null
//   GameText.description(q, stage = 0)   the description entry of that stage (0 = the opening text) or null;
//                                        falls back to the nearest lower stage
//   GameText.descriptions(q)             [{ stage, text }] all entries, in the game's order
//   GameText.objectives(q)               [string] objective titles
//   GameText.character(guid)             { name, descriptions: [{ stage, text }] } or null
const GameText = (() => {
  const empty = () => ({ found: false, by_guid: {}, guid_of: {}, characters: {} });
  let data = empty();
  const stageList = pairs => (pairs || []).map(([stage, text]) => ({ stage, text }));
  const HEX32 = /^[0-9a-f]{32}$/i;

  const api = {
    get found() { return data.found; },
    async load(lang, gameDir) {
      try {
        const invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
        data = invoke ? await invoke('game_texts', { lang, gameDir: gameDir || null }) : empty();
      } catch (_) { data = empty(); }
      return data.found;
    },
    guid(q) {
      if (!q) return null;
      const g = q.journal_guid || (HEX32.test(q.journal || '') ? q.journal : null) || data.guid_of[q.id];
      return g ? String(g).toLowerCase() : null;
    },
    entry(q) { const g = api.guid(q); return g ? data.by_guid[g] || null : null; },
    title(q) { const e = api.entry(q); return e && e.title ? e.title : null; },
    descriptions(q) { const e = api.entry(q); return e ? stageList(e.descriptions) : []; },
    description(q, stage = 0) {
      const all = api.descriptions(q);
      if (!all.length) return null;
      const best = all.filter(d => d.stage <= stage).pop() || all[0];
      return best.text;
    },
    objectives(q) { const e = api.entry(q); return e ? e.objectives.slice() : []; },
    character(guid) {
      const c = data.characters[String(guid || '').toLowerCase()];
      return c ? { name: c.name, descriptions: stageList(c.descriptions) } : null;
    },
  };
  return api;
})();
