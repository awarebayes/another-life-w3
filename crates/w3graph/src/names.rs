//! Official names read from the player's own install: the string ids the journal resources
//! (`.journal`, `CJournalQuest.title`, `CJournalCharacter.name`, ...) reference, resolved through
//! [`crate::strings`]. Nothing is stored in the repository.
use std::collections::HashMap;
use std::path::Path;

use crate::bundle;
use crate::cr2w::{self, Val};
use crate::quest::Error;
use crate::strings::{self, Strings};

/// One journal entry's localized property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalString {
    /// `CJournalQuest`, `CJournalCharacter`, ...
    pub class: String,
    pub base_name: String,
    /// `title`, `name`, `description`, ...
    pub prop: String,
    pub id: u32,
}

/// Every `LocalizedString` property of every export of every `.journal` file of the install.
pub fn journal_strings(install: &Path) -> Result<Vec<JournalString>, Error> {
    let mut out = Vec::new();
    for b in bundle::bundles_of_install(install) {
        let Ok(toc) = bundle::toc(&b) else { continue };
        for e in toc.iter().filter(|e| e.name.to_ascii_lowercase().ends_with(".journal")) {
            let Ok(d) = bundle::extract(&b, e) else { continue };
            let Ok(c) = cr2w::parse(&d, &e.name) else { continue };
            for x in &c.exports {
                for p in &x.props {
                    if p.ty != "LocalizedString" {
                        continue;
                    }
                    if let Val::Raw(r) = &p.value {
                        if let Ok(a) = <[u8; 4]>::try_from(&r[..r.len().min(4)]) {
                            out.push(JournalString { class: x.class.clone(), base_name: x.str_prop("baseName").unwrap_or("").to_owned(), prop: p.name.clone(), id: u32::from_le_bytes(a) });
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The distinct string ids the journals reference (what [`Strings::calibrate`] needs).
pub fn journal_ids(install: &Path) -> Result<Vec<u32>, Error> {
    let mut v: Vec<u32> = journal_strings(install)?.into_iter().map(|j| j.id).collect();
    v.sort_unstable();
    v.dedup();
    Ok(v)
}

/// One description paragraph of a journal quest or character.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DescText {
    /// The entry's GUID (32 lower-case hex digits).
    pub guid: String,
    /// `index` of the description entry: which stage of the quest (or of the character's story) shows
    /// this text; 0 when the field is absent. The game picks the entry matching the journal's progress,
    /// so several entries are stages, not one continuous text.
    pub index: u8,
    pub text: String,
}

/// One objective of a journal quest (`CJournalQuestObjective.title`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObjectiveText {
    pub guid: String,
    pub title: String,
}

/// A journal quest read from the install, texts in the asked language (empty where the string file has
/// no text for the id).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuestText {
    /// The `CJournalQuest` GUID: the first element of a journal path in a save (32 lower-case hex digits).
    pub guid: String,
    /// The `.journal` resource, e.g. `gameplay\journal\quests\mq0005dwarfsmith.journal`.
    pub resource: String,
    /// The English working name (`baseName`).
    pub base_name: String,
    pub title: String,
    /// The entries of the quest's `CJournalQuestDescriptionGroup`, in tree order.
    pub descriptions: Vec<DescText>,
    /// The objectives of all the quest's phases, in tree order.
    pub objectives: Vec<ObjectiveText>,
}

/// A `CJournalCharacter` read from the install.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterText {
    pub guid: String,
    pub resource: String,
    pub base_name: String,
    pub name: String,
    pub descriptions: Vec<DescText>,
}

/// The quests and characters of the install's journals in one language.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JournalTexts {
    pub quests: Vec<QuestText>,
    pub characters: Vec<CharacterText>,
}

fn ls_id(x: &cr2w::Export, prop: &str) -> Option<u32> {
    match x.prop(prop)? {
        Val::Raw(r) => <[u8; 4]>::try_from(r.get(..4)?).ok().map(u32::from_le_bytes).filter(|&i| i != 0),
        _ => None,
    }
}

/// Journal skeleton: the structs with empty texts and, per struct, its string ids in the order
/// title/name, descriptions, objectives.
struct Skeleton {
    quests: Vec<(QuestText, Vec<u32>)>,
    characters: Vec<(CharacterText, Vec<u32>)>,
}

fn desc_of(de: &cr2w::Export) -> Option<(DescText, u32)> {
    let id = ls_id(de, "description")?;
    let g = de.guid()?;
    Some((DescText { guid: cr2w::hex(&g), index: de.int_prop("index").map_or(0, |v| u8::try_from(v).unwrap_or(0)), text: String::new() }, id))
}

fn skeleton(install: &Path) -> Skeleton {
    let mut sk = Skeleton { quests: Vec::new(), characters: Vec::new() };
    let mut seen = std::collections::HashSet::new();
    for b in bundle::bundles_of_install(install) {
        let Ok(toc) = bundle::toc(&b) else { continue };
        for e in toc.iter().filter(|e| e.name.to_ascii_lowercase().ends_with(".journal")) {
            let Ok(d) = bundle::extract(&b, e) else { continue };
            let Ok(c) = cr2w::parse(&d, &e.name) else { continue };
            let kids = |i: usize| c.exports.iter().enumerate().filter(move |(_, k)| k.parent == Some(i));
            for (i, x) in c.exports.iter().enumerate() {
                let Some(g) = x.guid().map(|g| cr2w::hex(&g)) else { continue };
                let base_name = x.str_prop("baseName").unwrap_or("").to_owned();
                match x.class.as_str() {
                    "CJournalQuest" if seen.insert(g.clone()) => {
                        let mut q = QuestText { guid: g, resource: e.name.clone(), base_name, ..Default::default() };
                        let (mut dids, mut oids) = (Vec::new(), Vec::new());
                        for (ki, k) in kids(i) {
                            match k.class.as_str() {
                                "CJournalQuestDescriptionGroup" => {
                                    for (d, id) in kids(ki).filter_map(|(_, de)| desc_of(de)) {
                                        q.descriptions.push(d);
                                        dids.push(id);
                                    }
                                }
                                "CJournalQuestPhase" => {
                                    for (_, ob) in kids(ki).filter(|(_, ob)| ob.class == "CJournalQuestObjective") {
                                        if let (Some(id), Some(og)) = (ls_id(ob, "title"), ob.guid()) {
                                            q.objectives.push(ObjectiveText { guid: cr2w::hex(&og), title: String::new() });
                                            oids.push(id);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        let mut ids = vec![ls_id(x, "title").unwrap_or(0)];
                        ids.extend(dids);
                        ids.extend(oids);
                        sk.quests.push((q, ids));
                    }
                    "CJournalCharacter" if seen.insert(g.clone()) => {
                        let mut ch = CharacterText { guid: g, resource: e.name.clone(), base_name, ..Default::default() };
                        let mut ids = vec![ls_id(x, "name").unwrap_or(0)];
                        for (d, id) in kids(i).filter(|(_, k)| k.class == "CJournalCharacterDescription").filter_map(|(_, de)| desc_of(de)) {
                            ch.descriptions.push(d);
                            ids.push(id);
                        }
                        sk.characters.push((ch, ids));
                    }
                    _ => {}
                }
            }
        }
    }
    sk
}

/// Every journal quest (title, description entries, objective titles) and character (name,
/// description entries) of the install, with the texts of `lang`. Read at run time from the player's
/// files and kept in memory only (rule 7: no game text in the repository).
pub fn journal_texts(install: &Path, lang: &str) -> Result<JournalTexts, Error> {
    let sk = skeleton(install);
    let mut ids: Vec<u32> = sk.quests.iter().flat_map(|q| q.1.iter().copied()).chain(sk.characters.iter().flat_map(|c| c.1.iter().copied())).filter(|&i| i != 0).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut s = Strings::open(&strings::path_of(install, lang))?;
    s.calibrate(&ids).map_err(Error::Other)?;
    let tx = |i: u32| if i == 0 { String::new() } else { s.get(i).unwrap_or_default() };
    let mut out = JournalTexts::default();
    for (mut q, ids) in sk.quests {
        let mut it = ids.into_iter();
        q.title = tx(it.next().unwrap_or(0));
        for d in &mut q.descriptions {
            d.text = tx(it.next().unwrap_or(0));
        }
        for o in &mut q.objectives {
            o.title = tx(it.next().unwrap_or(0));
        }
        out.quests.push(q);
    }
    for (mut ch, ids) in sk.characters {
        let mut it = ids.into_iter();
        ch.name = tx(it.next().unwrap_or(0));
        for d in &mut ch.descriptions {
            d.text = tx(it.next().unwrap_or(0));
        }
        out.characters.push(ch);
    }
    Ok(out)
}

/// Lower-case, letters and digits only: how two spellings of a name are compared.
pub fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Journal quest GUIDs for English quest titles (matched on the English official title, [`norm`]alized;
/// `en` is [`journal_texts`] in English). A title that matches no quest, or several quests with
/// descriptions, is left out.
pub fn quest_guids_by_title(en: &JournalTexts, titles: &[String]) -> HashMap<String, String> {
    let mut by: HashMap<String, Vec<&QuestText>> = HashMap::new();
    for q in en.quests.iter().filter(|q| !q.title.is_empty() && !q.descriptions.is_empty()) {
        by.entry(norm(&q.title)).or_default().push(q);
    }
    titles.iter().filter_map(|t| match by.get(&norm(t))?.as_slice() {
        [q] => Some((t.clone(), q.guid.clone())),
        _ => None,
    }).collect()
}

/// Official names in `lang` for English names we use: each `english` name is found among the English
/// texts of the journal titles/names (`classes`), and the text of the same id in `lang` is returned.
/// Keys are the English names as given. Names that are ambiguous or absent are left out.
pub fn names_in(install: &Path, lang: &str, english: &[String], classes: &[&str]) -> Result<HashMap<String, String>, Error> {
    let strs = journal_strings(install)?;
    let mut ids: Vec<u32> = strs.iter().map(|j| j.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut en = Strings::open(&strings::path_of(install, "en"))?;
    en.calibrate(&ids).map_err(Error::Other)?;
    let mut tr = if lang == "en" { None } else { Some(Strings::open(&strings::path_of(install, lang))?) };
    if let Some(t) = tr.as_mut() {
        t.calibrate(&ids).map_err(Error::Other)?;
    }
    let mut by_name: HashMap<String, Vec<u32>> = HashMap::new();
    for j in strs.iter().filter(|j| classes.contains(&j.class.as_str()) && (j.prop == "title" || j.prop == "name")) {
        if let Some(t) = en.get(j.id) {
            let v = by_name.entry(norm(&t)).or_default();
            if !v.contains(&j.id) {
                v.push(j.id);
            }
        }
    }
    let mut out = HashMap::new();
    for name in english {
        let Some(ids) = by_name.get(&norm(name)) else { continue };
        // several ids with the same English text: take them only when the target texts agree
        let texts: Vec<String> = ids.iter().filter_map(|&i| tr.as_ref().map_or_else(|| en.get(i), |t| t.get(i))).collect();
        if let Some(first) = texts.first() {
            if texts.iter().all(|t| t == first) {
                out.insert(name.clone(), first.clone());
            }
        }
    }
    Ok(out)
}

/// Texts in `lang` of these game string ids (journal titles: `CJournalQuest.title`, the side quests' `title_id`).
/// Ids the strings file does not hold are left out.
pub fn texts_by_id(install: &Path, lang: &str, wanted: &[u32]) -> Result<HashMap<u32, String>, Error> {
    let ids = journal_ids(install)?;
    let mut s = Strings::open(&strings::path_of(install, lang))?;
    s.calibrate(&ids).map_err(Error::Other)?;
    Ok(wanted.iter().filter_map(|&i| s.get(i).filter(|t| !t.trim().is_empty()).map(|t| (i, t))).collect())
}

/// Official names in `lang` for item names as the save stores them (the `name` of the item
/// definition). The key-hash of the XML's `localisation_key_name` is not known (see [`crate::strings`]),
/// so an item is matched by its English display text among the strings that have a textual key: the
/// internal name and the display text are the same for many items ("Bread", "Dol Blathanna longsword")
/// and differ for others, which are then left out. A name whose candidates disagree in `lang` is left out.
pub fn item_names_in(install: &Path, lang: &str, items: &[String]) -> Result<HashMap<String, String>, Error> {
    let mut ids = journal_ids(install)?;
    ids.sort_unstable();
    let mut en = Strings::open(&strings::path_of(install, "en"))?;
    en.calibrate(&ids).map_err(Error::Other)?;
    let mut tr = if lang == "en" { None } else { Some(Strings::open(&strings::path_of(install, lang))?) };
    if let Some(t) = tr.as_mut() {
        t.calibrate(&ids).map_err(Error::Other)?;
    }
    let mut by_name: HashMap<String, Vec<u32>> = HashMap::new();
    for (id, text) in en.named() {
        by_name.entry(norm(&text)).or_default().push(id);
    }
    let mut out = HashMap::new();
    for name in items {
        // "Swallow 1" is the first (only) grade of the potion the game shows as "Swallow"
        let base = name.strip_suffix(" 1").unwrap_or(name);
        let Some(cands) = by_name.get(&norm(name)).or_else(|| by_name.get(&norm(base))) else { continue };
        let texts: Vec<String> = cands.iter().filter_map(|&i| tr.as_ref().map_or_else(|| en.get(i), |t| t.get(i))).collect();
        if let Some(first) = texts.first() {
            if texts.iter().all(|t| t == first) {
                out.insert(name.clone(), first.clone());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the install; skipped without one. Asserts counts and non-ASCII text only, never the text.
    #[test]
    fn journal_texts_resolve_in_russian_and_english() {
        let install = std::env::var("W3AL_INSTALL").unwrap_or_else(|_| r"C:\Program Files (x86)\Steam\steamapps\common\The Witcher 3".to_owned());
        let install = Path::new(&install);
        if !strings::path_of(install, "ru").is_file() || bundle::bundles_of_install(install).is_empty() {
            return;
        }
        let ru = journal_texts(install, "ru").expect("ru");
        let en = journal_texts(install, "en").expect("en");
        println!("quests {} chars {}", ru.quests.len(), ru.characters.len());
        assert!(ru.quests.len() > 300 && ru.characters.len() > 80);
        assert_eq!(ru.quests.len(), en.quests.len());
        let titled = ru.quests.iter().filter(|q| !q.title.is_empty()).count();
        let with_desc = ru.quests.iter().filter(|q| q.descriptions.iter().any(|d| !d.text.is_empty())).count();
        let obj: usize = ru.quests.iter().map(|q| q.objectives.iter().filter(|o| !o.title.is_empty()).count()).sum();
        println!("ru: titled {titled}, with description {with_desc}, objectives {obj}");
        assert!(titled > 300 && with_desc > 300 && obj > 1000);
        // Cyrillic text differs from the English one
        let cyr = ru.quests.iter().filter(|q| q.title.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c))).count();
        assert!(cyr * 10 > titled * 8);
        let named = ru.characters.iter().filter(|c| !c.name.is_empty()).count();
        assert!(named > 80);
        let want = vec!["The Last Wish".to_owned(), "Family Matters".to_owned(), "No Such Quest".to_owned()];
        let g = quest_guids_by_title(&en, &want);
        assert_eq!(g.len(), 2, "{g:?}");
        assert!(g.values().all(|v| v.len() == 32));
    }

    #[test]
    fn norm_ignores_case_and_punctuation() {
        assert_eq!(norm("King's Gambit"), norm("kings gambit"));
    }

    /// Needs the install (`W3AL_INSTALL`, else the Steam default); skipped without one.
    #[test]
    fn quest_titles_resolve_in_russian() {
        let install = std::env::var("W3AL_INSTALL").unwrap_or_else(|_| r"C:\Program Files (x86)\Steam\steamapps\common\The Witcher 3".to_owned());
        let install = Path::new(&install);
        if !strings::path_of(install, "ru").is_file() || bundle::bundles_of_install(install).is_empty() {
            return;
        }
        let want = vec!["The Last Wish".to_owned(), "Family Matters".to_owned(), "No Such Quest".to_owned()];
        let m = names_in(install, "ru", &want, &["CJournalQuest"]).expect("names");
        assert!(!m.contains_key("No Such Quest"));
        for q in ["The Last Wish", "Family Matters"] {
            let t = m.get(q).unwrap_or_else(|| panic!("{q} not resolved"));
            assert!(!t.is_ascii(), "{q} -> {t}");
        }
    }
}
