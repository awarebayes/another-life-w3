//! The quest list of the player's own install: every `CJournalQuest` of every `.journal` resource
//! (`gameplay\journal\quests\*.journal`, `dlc\*\journal\quests\*.journal`), with the group it sits
//! under, its type, world, title string id and (for contracts, races, fist fights) its phase file, and
//! the quest-graph blocks that write its journal entries. Nothing of it is stored in the repository:
//! the side-quest generator (`w3al gen-side-quests`) keeps only GUIDs, internal names and paths.
//!
//! `CJournalQuest` (read from the 5.01 files, `w3al res-dump`): `guid`, `baseName` (internal name),
//! `parentGuid` (the `CJournalQuestGroup` above it: Chapter 1, Minorquests, Treasure Hunt, ...),
//! `type: eQuestType` (an enum, stored as an index into the file's name table), `world: Uint32`,
//! `title: LocalizedString` (u32 string id), optional `questPhase` (soft handle to the quest's
//! `.w2phase`) and `huntingQuestPath`. The save's journal path of a quest's root entry is
//! `[group GUID, quest GUID]` (e.g. `questact1/q401_hunt_with_eskel`).
use std::collections::HashMap;

use crate::cr2w::{self, Val};
use crate::quest::{normalize, BlockKind, Error, Guid, Library};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalQuest {
    pub guid: Guid,
    /// `parentGuid`: the quest group (the first step of the save's journal path).
    pub group: Guid,
    /// The group's internal name (`Chapter 1`, `Minorquests`, ...); empty when the group file is not found.
    pub group_name: String,
    /// The `.journal` resource (normalized).
    pub file: String,
    /// `baseName`: the internal name (not shown in game).
    pub base_name: String,
    /// `title`: the localized string id of the official title (resolved at run time, `crate::names`).
    pub title_id: Option<u32>,
    /// `type` (`eQuestType`): `QuestType_Story`, `QuestType_Chapter`, `QuestType_Side`, `QuestType_MonsterHunt`,
    /// `QuestType_TreasureHunt`, ... as the file's name table spells it.
    pub qtype: String,
    /// `world`: the map area id.
    pub world: u32,
    /// `questPhase`: the quest's own phase file (contracts, races, fist fights, some side quests).
    pub phase_file: Option<String>,
}

impl JournalQuest {
    /// The save's journal key of the quest's root entry.
    pub fn root_key(&self) -> Vec<Guid> {
        vec![self.group, self.guid]
    }
}

fn enum_name(c: &cr2w::Cr2w, v: Option<&Val>) -> String {
    match v {
        Some(Val::Raw(b)) if b.len() >= 2 => {
            let i = u16::from_le_bytes([b[0], b[1]]) as usize;
            c.names.get(i).cloned().unwrap_or_default()
        }
        Some(Val::Name(s)) => s.clone(),
        _ => String::new(),
    }
}

fn u32_of(v: Option<&Val>) -> Option<u32> {
    match v {
        Some(Val::Int(i)) => u32::try_from(*i).ok(),
        Some(Val::Raw(b)) if b.len() >= 4 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => None,
    }
}

/// Every quest of every `.journal` resource the library knows (`Library::load_install` registers them).
pub fn journal_quests(lib: &Library) -> Result<Vec<JournalQuest>, Error> {
    let mut quests = Vec::new();
    let mut groups: HashMap<Guid, String> = HashMap::new();
    for name in lib.resource_names(".journal") {
        let Ok(d) = lib.read_resource(&name) else { continue };
        let Ok(c) = cr2w::parse(&d, &name) else { continue };
        for e in &c.exports {
            match e.class.as_str() {
                "CJournalQuestGroup" => {
                    if let Some(g) = e.guid() {
                        groups.insert(g, e.str_prop("baseName").unwrap_or("").to_owned());
                    }
                }
                "CJournalQuest" => {
                    let (Some(guid), Some(Val::Guid(group))) = (e.guid(), e.prop("parentGuid")) else { continue };
                    quests.push(JournalQuest {
                        guid,
                        group: *group,
                        group_name: String::new(),
                        file: name.clone(),
                        base_name: e.str_prop("baseName").unwrap_or("").to_owned(),
                        title_id: u32_of(e.prop("title")),
                        qtype: enum_name(&c, e.prop("type")),
                        world: u32_of(e.prop("world")).unwrap_or(0),
                        phase_file: e.str_prop("questPhase").filter(|s| !s.is_empty()).map(normalize),
                    });
                }
                _ => {}
            }
        }
    }
    for q in &mut quests {
        q.group_name = groups.get(&q.group).cloned().unwrap_or_default();
    }
    Ok(quests)
}

/// Quest GUID -> the journal blocks (`CJournalQuestBlock`) whose path goes through that quest
/// (`path[1]`), in library order.
pub fn journal_blocks_by_quest(lib: &Library) -> HashMap<Guid, Vec<usize>> {
    let mut m: HashMap<Guid, Vec<usize>> = HashMap::new();
    for (i, b) in lib.blocks.iter().enumerate() {
        if let BlockKind::Journal { quest: true, path } = &b.kind {
            if let Some(q) = path.get(1) {
                m.entry(q.guid).or_default().push(i);
            }
        }
    }
    m
}
