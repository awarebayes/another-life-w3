//! Side resources the synthesis needs besides the quest graphs: spawn sets (`.w2comm`) and
//! journal definitions (`.journal`). Both are `CR2W` files read with [`crate::cr2w`].
//!
//! `.w2comm` (`CCommunity`): `communityTable: array of CSTableEntry { entities, storyPhases: array of
//! CSStoryPhaseEntry { guid, storyPhaseName: { tags: TagList }, timetableName, .. }, guid }`. The save's
//! `community > communities > c[<file>] > communityData` mirrors it: one `tableEntry[GUID]` per table
//! entry with one `phaseEntry[GUID]` per story phase, in file order (checked on `q105_boy1.w2comm`
//! against the spirit-released donor: 1 table, 6 phases, same order). A stub's
//! `storyPhaseLocation > storyEntry` is one of these phase GUIDs; its phase name is the first tag of
//! `storyPhaseName` (Keira's `bfbf0fab` -> `q109_island_laboratory`, `59579094` ->
//! `q109_island_attacking`, the two values of `activePhase` around the Keira decision).
//!
//! `.journal` (`CJournalResource`): a tree of `CJournalQuest > CJournalQuestPhase > CJournalQuestObjective`
//! (and description groups / entries, map pins). The save's `JObjectiveCounters` holds one counter per
//! objective entry in the journal: 214 of 214 objective entries of T_keira have one, and the three
//! counters without an objective belong to tutorial entries. So a new objective entry gets `count 0`
//! (the KM/Radovid donors: `b47bfb41`; the spirit donors: `bf144fbd`; Letho: none, its new entries are
//! description entries).
use std::collections::HashSet;

use crate::cr2w::{self, Cr2w, Val};
use crate::quest::{tag_list, Error, Guid};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommPhase {
    pub guid: Guid,
    /// The story phase name (first tag of `storyPhaseName`), `""` for the unnamed default phase.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommTable {
    pub guid: Guid,
    pub phases: Vec<CommPhase>,
}

/// One spawn set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Community {
    pub file: String,
    pub tables: Vec<CommTable>,
}

impl Community {
    /// The phase name a story-phase entry GUID belongs to.
    pub fn phase_of(&self, story_entry: &Guid) -> Option<&str> {
        self.tables.iter().flat_map(|t| &t.phases).find(|p| &p.guid == story_entry).map(|p| p.name.as_str())
    }

    pub fn phase_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for p in self.tables.iter().flat_map(|t| &t.phases) {
            if !v.contains(&p.name.as_str()) {
                v.push(&p.name);
            }
        }
        v
    }
}

fn guid_of(props: &[cr2w::Prop]) -> Option<Guid> {
    match cr2w::prop_of(props, "guid") {
        Some(Val::Guid(g)) => Some(*g),
        _ => None,
    }
}

/// Parse a `.w2comm` file.
pub fn parse_community(data: &[u8], name: &str) -> Result<Community, Error> {
    let c: Cr2w = cr2w::parse(data, name)?;
    let root = c.exports.iter().find(|e| e.class == "CCommunity").ok_or_else(|| Error::Format(format!("{name}: no CCommunity export")))?;
    let mut out = Community { file: name.to_owned(), tables: Vec::new() };
    let Some(Val::Structs(tables)) = root.prop("communityTable") else { return Ok(out) };
    for t in tables {
        let Some(guid) = guid_of(t) else { continue };
        let mut phases = Vec::new();
        if let Some(Val::Structs(sp)) = cr2w::prop_of(t, "storyPhases") {
            for p in sp {
                let Some(pg) = guid_of(p) else { continue };
                let pname = match cr2w::prop_of(p, "storyPhaseName") {
                    Some(Val::Struct(n)) => tag_list(&c, cr2w::prop_of(n, "tags")).into_iter().next().unwrap_or_default(),
                    _ => String::new(),
                };
                phases.push(CommPhase { guid: pg, name: pname });
            }
        }
        out.tables.push(CommTable { guid, phases });
    }
    Ok(out)
}

/// The GUIDs of every `CJournalQuestObjective` in a `.journal` file.
pub fn journal_objectives(data: &[u8], name: &str) -> Result<HashSet<Guid>, Error> {
    let c = cr2w::parse(data, name)?;
    Ok(c.exports.iter().filter(|e| e.class == "CJournalQuestObjective").filter_map(|e| e.guid()).collect())
}
