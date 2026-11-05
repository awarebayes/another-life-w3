//! The "start kit": the level, crowns and a modest level-appropriate kit the player starts with, written into
//! the exported save after the story effects (`[start_kit]` in the run file).
//!
//! All format knowledge lives in `w3save::character`; this module only decides *what* to write and
//! checks the written file. What is proven and what is a hypothesis: docs/findings/character-inventory.md.
use serde::{Deserialize, Serialize};
use w3save::character::{self, ItemForm, NewItem};
use w3save::{Container, Document};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gear {
    /// Keep the template's inventory as it is.
    #[default]
    None,
    /// Add the [`kit_for`] items the template does not carry yet (sword pair, armour set, potions, oil, bomb)
    /// and top up the supply stacks it does.
    Recommended,
}

/// The `[start_kit]` section of a run file. Every part is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartKit {
    /// Geralt's level, 1 ..= 50 (the saved XP table's range).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    /// Crowns in the purse (above 65535 the record's quantity is widened to a u32, see `set_item_quantity`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crowns: Option<u32>,
    #[serde(default)]
    pub gear: Gear,
    /// Single items of the recommended table to add by name (edit mode): each must be in [`kit_for`] the
    /// target level; supply stacks are raised, missing records created, carried ones left alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
}

impl StartKit {
    pub fn is_empty(&self) -> bool {
        self.level.is_none() && self.crowns.is_none() && self.gear == Gear::None && self.items.is_empty()
    }

    /// Kit entries this kit writes for `level`: the whole table for `Recommended`, else the named items.
    fn wanted(&self, level: u32) -> Vec<KitItem> {
        if self.gear == Gear::Recommended {
            return kit_for(level);
        }
        kit_for(level).into_iter().filter(|k| self.items.iter().any(|n| n == k.name)).collect()
    }
}

/// One kit entry: the item name as the save stores it (the `name` of the item definition in the game's
/// `gameplay\items\*.xml`), how many, and the record shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KitItem {
    pub name: &'static str,
    pub quantity: u16,
    pub form: ItemForm,
}

const fn gear(name: &'static str, weapon: bool, slots: u8) -> KitItem {
    KitItem { name, quantity: 1, form: ItemForm::Gear { weapon, durability: 100.0, slots } }
}
const fn charged(name: &'static str, charges: u32) -> KitItem {
    KitItem { name, quantity: 1, form: ItemForm::Charged { charges } }
}
const fn stack(name: &'static str, quantity: u16) -> KitItem {
    KitItem { name, quantity, form: ItemForm::Stack }
}

/// Consumables of every band: tier-1 potions, oil and bomb with the charges the game wrote on them in the
/// fixtures (`Cat 1` / `Swallow 1` ammo_current 3, `Beast Oil 1` 20, `Grapeshot 1` 2). No higher tier is
/// offered: the charges of tier 2 were never seen and tier 3 varies with skills.
const CONSUMABLES: [KitItem; 4] = [charged("Swallow 1", 3), charged("Cat 1", 3), charged("Beast Oil 1", 20), charged("Grapeshot 1", 2)];

/// Weapon / armour sets by level band: (first level of the band, sword pair + armour set). Names and
/// `enhancement_slots` are the item definitions' (`def_item_swords_dol_blathanna.xml`, `def_item_swords_vixen.xml`,
/// `def_item_armor_dol_blathanna.xml`, `def_item_crafting_weapons.xml`, `def_item_crafting_armor/boots/gloves/pants.xml`);
/// the level of each piece (7, 11, 18, 26, 34) is the lead from the third-party catalog, not read from the XML
/// (the XML only holds the stats). All are fixed-stat pieces (no `Autogen` tag), so no random ability list exists.
const GEAR_BANDS: &[(u32, [KitItem; 6])] = &[
    (1, [gear("Dol Blathanna longsword", true, 1), gear("Silver Vixen", true, 1), gear("Dol Blathanna Armor", false, 1), gear("Dol Blathanna Boots", false, 0), gear("Dol Blathanna Gloves", false, 0), gear("Dol Blathanna Pants", false, 0)]),
    (11, [gear("Gryphon School steel sword", true, 1), gear("Gryphon School silver sword", true, 1), gear("Gryphon Armor", false, 1), gear("Gryphon Boots 1", false, 0), gear("Gryphon Gloves 1", false, 0), gear("Gryphon Pants 1", false, 0)]),
    (18, [gear("Gryphon School steel sword 1", true, 2), gear("Gryphon School silver sword 1", true, 2), gear("Gryphon Armor 1", false, 2), gear("Gryphon Boots 2", false, 0), gear("Gryphon Gloves 2", false, 0), gear("Gryphon Pants 2", false, 1)]),
    (26, [gear("Gryphon School steel sword 2", true, 3), gear("Gryphon School silver sword 2", true, 3), gear("Gryphon Armor 2", false, 3), gear("Gryphon Boots 3", false, 1), gear("Gryphon Gloves 3", false, 1), gear("Gryphon Pants 3", false, 2)]),
    (34, [gear("Gryphon School steel sword 3", true, 3), gear("Gryphon School silver sword 3", true, 3), gear("Gryphon Armor 3", false, 3), gear("Gryphon Boots 4", false, 1), gear("Gryphon Gloves 4", false, 1), gear("Gryphon Pants 4", false, 2)]),
];

/// Supply stacks per band: raised when the template already carries them, created otherwise.
const SUPPLY_BANDS: &[(u32, [KitItem; 4])] = &[
    (1, [stack("Bread", 5), stack("Blunt Bolt", 20), stack("weapon_repair_kit_1", 2), stack("armor_repair_kit_1", 2)]),
    (15, [stack("Bread", 8), stack("Blunt Bolt", 40), stack("weapon_repair_kit_2", 3), stack("armor_repair_kit_2", 3)]),
    (30, [stack("Bread", 10), stack("Blunt Bolt", 60), stack("weapon_repair_kit_3", 4), stack("armor_repair_kit_3", 4)]),
];

/// The whole recommended kit for `level`.
pub fn kit_for(level: u32) -> Vec<KitItem> {
    let mut out = Vec::new();
    if let Some((_, g)) = GEAR_BANDS.iter().rev().find(|(from, _)| level >= *from) {
        out.extend_from_slice(g);
    }
    out.extend_from_slice(&CONSUMABLES);
    if let Some((_, s)) = SUPPLY_BANDS.iter().rev().find(|(from, _)| level >= *from) {
        out.extend_from_slice(s);
    }
    out
}

/// Apply the kit to the document. Returns report lines (one per change or skip). All-or-nothing is the
/// caller's job (it discards the document on error).
pub fn apply(doc: &mut Document, kit: &StartKit) -> Result<Vec<String>, w3save::Error> {
    let mut report = Vec::new();
    if let Some(level) = kit.level {
        let level = i32::try_from(level).map_err(|_| w3save::Error::Unverified(format!("start kit: level {level} is out of range")))?;
        let c = character::set_level(doc, level)?;
        report.push(format!(
            "start kit: level {} -> {} (xp {}, skill points free {} used {}, +{} / -{} Lvl abilities, vitality {:+})",
            c.from,
            c.to,
            c.xp.used,
            c.skill.free,
            c.skill.used,
            c.added_abilities.len(),
            c.removed_abilities.len(),
            c.vitality_delta
        ));
    }
    if kit.gear == Gear::Recommended || !kit.items.is_empty() {
        let level = match kit.level {
            Some(l) => l,
            None => u32::try_from(character::read(doc)?.level.level).unwrap_or(1),
        };
        let wanted = kit.wanted(level);
        for n in &kit.items {
            if !wanted.iter().any(|k| k.name == n) {
                return Err(w3save::Error::Unverified(format!("start kit: `{n}` is not in the recommended table for level {level}")));
            }
        }
        for k in wanted {
            let have = character::read(doc)?;
            if have.items.iter().filter(|i| i.name == k.name).count() > 1 {
                report.push(format!("start kit: {} is carried in several stacks, left as it is", k.name));
                continue;
            }
            match (have.items.iter().find(|i| i.name == k.name), k.form) {
                (Some(i), ItemForm::Stack) if i.quantity < u32::from(k.quantity) => {
                    character::set_item_quantity(doc, k.name, u32::from(k.quantity))?;
                    report.push(format!("start kit: {} {} -> {}", k.name, i.quantity, k.quantity));
                }
                (Some(i), _) => report.push(format!("start kit: {} already carried ({})", k.name, i.quantity)),
                (None, form) => {
                    let uid = character::add_item(doc, &NewItem { name: k.name.to_owned(), quantity: k.quantity, form })?;
                    report.push(format!("start kit: added {} x{} (item id {uid})", k.name, k.quantity));
                }
            }
        }
    }
    if let Some(c) = kit.crowns {
        let before = character::read(doc)?.crowns();
        character::set_crowns(doc, c)?;
        report.push(format!("start kit: crowns {before:?} -> {c}"));
    }
    Ok(report)
}

/// Apply the kit to a finished save (`bytes`) and check the result: the new payload re-parses and
/// re-serializes identically, the recomputed variable table equals the one written, `check-save` finds no
/// error, the NM names only grew, no top-level block except `universe` (the player entity) changed, and
/// [`verify_bytes`] finds the kit in it. Returns the new bytes and the report lines.
pub fn apply_to_save(bytes: &[u8], kit: &StartKit) -> Result<(Vec<u8>, Vec<String>), String> {
    let e = |x: w3save::Error| x.to_string();
    let mut container = Container::read(bytes).map_err(e)?;
    let mut doc = Document::parse(&container.payload, container.data_start).map_err(e)?;
    let original = doc.clone();
    let report = apply(&mut doc, kit).map_err(e)?;
    container.payload = doc.to_bytes();
    let out = container.write().map_err(e)?;

    let mut problems = Vec::new();
    let re = Container::read(&out).map_err(e)?;
    let again = Document::parse(&re.payload, re.data_start).map_err(e)?;
    if again.to_bytes() != re.payload {
        problems.push("start kit: the typed re-parse does not serialize back to the payload".to_owned());
    }
    if again.table() != again.map.variables {
        problems.push("start kit: the variable table recomputed from the tree differs from the one written".to_owned());
    }
    match w3save::inspect(&out) {
        Ok(info) => problems.extend(info.issues.iter().filter(|i| i.severity == w3save::Severity::Error).map(|i| format!("start kit: check-save: {}", i.message))),
        Err(x) => problems.push(format!("start kit: check-save: {x}")),
    }
    if !w3save::graph::names_prefix(&original, &again) {
        problems.push("start kit: the NM names are not a prefix of the result's".to_owned());
    }
    let (a, b): (Vec<_>, Vec<_>) = (original.top_nodes().collect(), again.top_nodes().collect());
    if a.len() != b.len() {
        problems.push("start kit: the top-level node count changed".to_owned());
    } else {
        for (x, y) in a.iter().zip(&b) {
            let name = w3save::graph::node_name(&original, x);
            if matches!(x.kind, w3save::Kind::Manu(_)) || name == "universe" {
                continue;
            }
            if x.to_bytes() != y.to_bytes() {
                problems.push(format!("start kit: top-level {name} changed"));
            }
        }
    }
    problems.extend(verify_bytes(bytes, &out, kit).map_err(e)?.into_iter().map(|p| format!("start kit: {p}")));
    if problems.is_empty() {
        Ok((out, report))
    } else {
        Err(problems.join("; "))
    }
}

fn doc_of(bytes: &[u8]) -> Result<Document, w3save::Error> {
    let c = Container::read(bytes)?;
    Document::parse(&c.payload, c.data_start)
}

/// Re-read an exported save and compare it with its template: what the kit asked for must be there and
/// nothing else of the character may have changed. Returns the problems found (empty = fine).
pub fn verify_bytes(template: &[u8], written: &[u8], kit: &StartKit) -> Result<Vec<String>, w3save::Error> {
    let (a, b) = (character::read(&doc_of(template)?)?, character::read(&doc_of(written)?)?);
    let mut problems = Vec::new();
    if let Some(l) = kit.level {
        if b.level.level != l as i32 {
            problems.push(format!("level is {} not {l}", b.level.level));
        }
        if Some(b.level.xp.used) != b.level.exp_for_level(l as i32) || b.level.xp.free != 0 {
            problems.push("experience pool does not match the level".into());
        }
    } else if a.level != b.level {
        problems.push("the level data changed although no level was asked for".into());
    }
    if let Some(c) = kit.crowns {
        if b.crowns() != Some(c) {
            problems.push(format!("crowns are {:?} not {c}", b.crowns()));
        }
    } else if a.crowns() != b.crowns() {
        problems.push("crowns changed although none were asked for".into());
    }
    let wanted = kit.wanted(kit.level.unwrap_or(a.level.level as u32));
    // Every old item is still there, unchanged unless it is a kit stack or the crowns.
    // (New records are appended, so the old ones keep their order and position.)
    if b.items.len() < a.items.len() {
        problems.push("item records were removed".into());
    }
    for (x, y) in a.items.iter().zip(&b.items) {
        if x.name != y.name {
            problems.push(format!("item {} moved or changed to {}", x.name, y.name));
            continue;
        }
        let allowed = (kit.crowns.is_some() && x.name == "Crowns") || wanted.iter().any(|k| k.name == x.name && k.form == ItemForm::Stack);
        if (!allowed && x.quantity != y.quantity) || x.durability.to_bits() != y.durability.to_bits() {
            problems.push(format!("item {} changed unexpectedly", x.name));
        }
    }
    // Every kit item is there; the other new records are exactly those.
    for k in &wanted {
        match b.items.iter().find(|y| y.name == k.name) {
            None => problems.push(format!("kit item {} is missing", k.name)),
            Some(y) if matches!(k.form, ItemForm::Gear { durability, .. } if y.durability != durability) => problems.push(format!("kit item {} has the wrong durability", k.name)),
            Some(y) if matches!(k.form, ItemForm::Stack) && y.quantity < u32::from(k.quantity) => problems.push(format!("kit stack {} is below {}", k.name, k.quantity)),
            Some(_) => {}
        }
    }
    let added = b.items.len() - a.items.len().min(b.items.len());
    if added > wanted.len() {
        problems.push(format!("{added} records were added, the kit has {} items", wanted.len()));
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_pick_the_highest_lower_bound() {
        let names = |l| kit_for(l).iter().map(|k| k.name).collect::<Vec<_>>();
        assert_eq!(names(1)[0], "Dol Blathanna longsword");
        assert_eq!(names(10)[0], "Dol Blathanna longsword");
        assert_eq!(names(11)[0], "Gryphon School steel sword");
        assert_eq!(names(20)[0], "Gryphon School steel sword 1");
        assert_eq!(names(50)[0], "Gryphon School steel sword 3");
        assert!(names(14).contains(&"weapon_repair_kit_1") && names(15).contains(&"weapon_repair_kit_2"));
        // one steel + one silver sword, a full armour set, 4 consumables, 4 supplies at every level
        for l in [1, 11, 18, 26, 34, 50] {
            assert_eq!(kit_for(l).len(), 6 + 4 + 4);
        }
    }

    #[test]
    fn section_round_trips_through_toml() {
        let kit: StartKit = toml::from_str("level = 12\ncrowns = 800\ngear = \"recommended\"\n").unwrap();
        assert_eq!(kit, StartKit { level: Some(12), crowns: Some(800), gear: Gear::Recommended, items: vec![] });
        assert_eq!(toml::from_str::<StartKit>(&toml::to_string(&kit).unwrap()).unwrap(), kit);
        assert!(StartKit::default().is_empty());
    }
}
