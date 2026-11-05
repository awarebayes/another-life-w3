'use strict';
// UI translation layer: plain JS, no build step. Our own interface strings live in i18n/<lang>.json
// (flat keys, `{name}` placeholders, plural forms as key.one / key.few / key.many / key.other).
// The questbook narration is separate: it stays English until `*_ru` fields exist (see loc()).
//
//   t('key', { n: 3 })     text, falls back to English, then to the key itself
//   t.has('key')           is there a text for it
//   tn('key', 3, vars)     plural form by the language's rules (Intl.PluralRules)
//   loc(obj, 'summary')    obj.summary_ru when the language is ru and the field exists, else obj.summary
//   I18N.lang, I18N.set('ru'), I18N.ready  (promise), I18N.apply() for [data-i18n] / [data-i18n-aria]
const I18N = (() => {
  const LANGS = { en: 'English', ru: 'Русский' };
  const KEY = 'w3al.lang';
  let lang = 'en';
  let dict = {};
  let base = {};

  function stored() {
    try { return window.localStorage.getItem(KEY); } catch (_) { return null; }
  }
  function remember(code) {
    try { window.localStorage.setItem(KEY, code); } catch (_) { /* private mode: the choice just lasts this session */ }
  }
  function systemLang() {
    const l = (navigator.languages && navigator.languages[0]) || navigator.language || 'en';
    const short = String(l).toLowerCase().slice(0, 2);
    return LANGS[short] ? short : 'en';
  }
  async function fetchDict(code) {
    try {
      const r = await fetch('i18n/' + code + '.json');
      if (!r.ok) return {};
      return await r.json();
    } catch (_) { return {}; }
  }
  function fill(s, vars) {
    return vars ? s.replace(/\{(\w+)\}/g, (m, k) => (vars[k] == null ? m : String(vars[k]))) : s;
  }

  const api = {
    LANGS,
    get lang() { return lang; },
    async set(code) {
      if (!LANGS[code]) code = 'en';
      if (!Object.keys(base).length) base = await fetchDict('en');
      dict = code === 'en' ? base : await fetchDict(code);
      lang = code;
      remember(code);
      document.documentElement.lang = code;
      api.apply();
    },
    apply() {
      document.querySelectorAll('[data-i18n]').forEach(el => { el.textContent = t(el.getAttribute('data-i18n')); });
      document.querySelectorAll('[data-i18n-aria]').forEach(el => { el.setAttribute('aria-label', t(el.getAttribute('data-i18n-aria'))); });
    },
    has(key) { return key in dict || key in base; },
    t(key, vars) {
      const s = key in dict ? dict[key] : (key in base ? base[key] : key);
      return fill(s, vars);
    },
    tn(key, n, vars) {
      let cat = 'other';
      try { cat = new Intl.PluralRules(lang).select(n); } catch (_) { /* keep other */ }
      const k = [key + '.' + cat, key + '.other', key].find(x => api.has(x)) || key;
      return api.t(k, Object.assign({ n }, vars));
    },
  };
  api.ready = api.set(stored() || systemLang());
  return api;
})();
const t = I18N.t;
t.has = I18N.has;
const tn = I18N.tn;
function loc(obj, field) {
  if (!obj) return '';
  const alt = I18N.lang === 'en' ? null : obj[field + '_' + I18N.lang];
  return alt || obj[field];
}
