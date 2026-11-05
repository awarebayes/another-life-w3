//! New Game+ state of a save, and the switch that makes a New Game+ save an ordinary game.
//!
//! Where the state lives (docs/findings/new-game-plus.md, read from 5.01 saves and the game's scripts):
//!
//! * fact `NewGamePlus` (value 1) - every script that scales anything for New Game+ asks
//!   `FactsQuerySum("NewGamePlus") > 0` (enemy level in `npc.ws`, dodge / parry chance, item level, XP, quest levels);
//! * fact `FinalNewGamePlusLevel` (the level Geralt had when the New Game+ began) - read once at load by
//!   `CR4GameParams` (`gameParams.ws`: `newGamePlusLevel = FactsQuerySum("FinalNewGamePlusLevel")`);
//! * the top-level block `NewGamePlus` with the bools `Enable` ("New Game+ is unlocked": `EnableNewGamePlus`) and
//!   `active` (HYPOTHESIS: this playthrough is a New Game+; native code, no script reads it by that name).
//!
//! A save that never was a New Game+ has neither fact and `active = false`. The game's own console command
//! `simngp(false)` (`temp.ws`) does `FactsRemove("NewGamePlus")`, nothing else: [`turn_off`] does that, removes
//! the level fact too and clears `active`.
//!
//! HYPOTHESIS (not tried in game): a save edited like this loads and plays as a normal game. Item records are NOT
//! touched (see [`State::ngp_items`]).
use crate::character;
use crate::document::Document;
use crate::error::{Error, Result};
use crate::token::{Kind, Value};

pub const FACT_FLAG: &str = "NewGamePlus";
pub const FACT_LEVEL: &str = "FinalNewGamePlusLevel";
pub const BLOCK: &str = "NewGamePlus";

/// What a save says about New Game+.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    /// `FactsQuerySum("NewGamePlus")`; 0 when the fact is absent.
    pub flag: i64,
    /// `FactsQuerySum("FinalNewGamePlusLevel")`; `None` when the fact is absent.
    pub level: Option<i64>,
    /// `NewGamePlus > Enable`.
    pub enable: bool,
    /// `NewGamePlus > active`.
    pub active: bool,
    /// Item records named `NGP ...` (the New Game+ variants of DLC items).
    pub ngp_items: usize,
}

impl State {
    /// The game would treat the save as a New Game+ (what the scripts test).
    pub fn is_new_game_plus(&self) -> bool {
        self.flag > 0
    }
    /// Nothing of New Game+ is left: neither fact, `active` false.
    pub fn is_clean(&self) -> bool {
        self.flag == 0 && self.level.is_none() && !self.active
    }
}

fn block_bool(doc: &Document, var: &str) -> Result<bool> {
    let ix = doc.name_index(var).ok_or_else(|| Error::Unverified(format!("no `{var}` name in the save")))?;
    for n in doc.top_nodes() {
        if let Kind::Block(b) = &n.kind {
            if doc.name(b.name) == Some(BLOCK) {
                for c in &b.children {
                    if let Kind::Var(v) = &c.kind {
                        if v.name == ix {
                            return match v.value {
                                Value::Bool(x) => Ok(x),
                                _ => Err(Error::Unverified(format!("{BLOCK} > {var} is not a Bool"))),
                            };
                        }
                    }
                }
                return Err(Error::Unverified(format!("{BLOCK} has no `{var}`")));
            }
        }
    }
    Err(Error::Unverified(format!("the save has no top-level `{BLOCK}` block")))
}

/// Read the New Game+ state.
pub fn read(doc: &Document) -> Result<State> {
    let facts = doc.facts_table()?;
    let ngp_items = character::read(doc).map(|c| c.items.iter().filter(|i| i.name.starts_with("NGP ")).count()).unwrap_or(0);
    Ok(State {
        flag: facts.get(FACT_FLAG).map_or(0, |f| f.sum()),
        level: facts.get(FACT_LEVEL).map(|f| f.sum()),
        enable: block_bool(doc, "Enable")?,
        active: block_bool(doc, "active")?,
        ngp_items,
    })
}

fn set_active(doc: &mut Document, value: bool) -> Result<()> {
    let ix = doc.name_index("active").ok_or_else(|| Error::Unverified("no `active` name in the save".into()))?;
    let block_ix = doc.name_index(BLOCK).ok_or_else(|| Error::Unverified(format!("no `{BLOCK}` name in the save")))?;
    let mut hit = 0;
    for n in doc.top_nodes_mut() {
        if let Kind::Block(b) = &mut n.kind {
            if b.name == block_ix {
                for c in &mut b.children {
                    if let Kind::Var(v) = &mut c.kind {
                        if v.name == ix {
                            if !matches!(v.value, Value::Bool(_)) {
                                return Err(Error::Unverified(format!("{BLOCK} > active is not a Bool")));
                            }
                            v.value = Value::Bool(value);
                            hit += 1;
                        }
                    }
                }
            }
        }
    }
    if hit == 1 {
        Ok(())
    } else {
        Err(Error::Unverified(format!("expected exactly one {BLOCK} > active, found {hit}")))
    }
}

/// What [`turn_off`] changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub before: State,
    pub after: State,
    pub removed_facts: Vec<String>,
}

/// Make a New Game+ save a normal game: remove the facts `NewGamePlus` and `FinalNewGamePlusLevel` and set
/// `NewGamePlus > active` to false. Nothing else is touched (item records, level, quests). A save that is already
/// clean is left byte-identical. On error the document must be discarded (the caller works on a copy).
pub fn turn_off(doc: &mut Document) -> Result<Change> {
    let before = read(doc)?;
    if before.is_clean() {
        return Ok(Change { after: before.clone(), before, removed_facts: Vec::new() });
    }
    let mut removed = Vec::new();
    doc.edit_facts(|t| {
        for name in [FACT_FLAG, FACT_LEVEL] {
            if t.remove(name).is_some() {
                removed.push(name.to_owned());
            }
        }
        Ok(())
    })?;
    if before.active {
        set_active(doc, false)?;
        doc.rebase()?;
    }
    let after = read(doc)?;
    if !after.is_clean() {
        return Err(Error::Unverified("New Game+ state is still there after turn_off".into()));
    }
    Ok(Change { before, after, removed_facts: removed })
}
