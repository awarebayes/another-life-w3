//! Fixture-gated: `w3save::character` (docs/findings/character-inventory.md). Skipped (passes) when
//! `W3AL_FIXTURES` is unset; `W3AL_EXTRA_FIXTURES` (optional, a second folder, e.g. the Nexus packs)
//! widens the set. Fixtures are copies of real saves and are never committed.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use w3save::character::{self, Character};
use w3save::{payload, Container, Document};

fn saves(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            saves(&p, out);
        } else if p.extension().is_some_and(|x| x == "sav") {
            out.push(p);
        }
    }
}

fn fixtures() -> Option<Vec<PathBuf>> {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return None;
    };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    if let Some(extra) = std::env::var_os("W3AL_EXTRA_FIXTURES") {
        saves(Path::new(&extra), &mut files);
    }
    files.sort();
    Some(files)
}

fn load(p: &Path) -> Option<Document> {
    let bytes = std::fs::read(p).ok()?;
    let c = Container::read(&bytes).ok()?;
    Document::parse(&c.payload, c.data_start).ok()
}

/// Every save whose character we can read, with the data.
fn readable(files: &[PathBuf]) -> Vec<(PathBuf, Document, Character)> {
    files
        .iter()
        .filter_map(|p| {
            let d = load(p)?;
            let c = character::read(&d).ok()?;
            Some((p.clone(), d, c))
        })
        .collect()
}

#[test]
fn the_player_record_stream_round_trips_on_every_fixture() {
    let Some(files) = fixtures() else { return };
    let (mut ok, mut total) = (0, 0);
    for p in &files {
        let Some(d) = load(p) else { continue };
        total += 1;
        // `player_stream` itself refuses a stream that does not re-serialize to the same bytes.
        match character::player_stream(&d) {
            Ok(_) => ok += 1,
            Err(e) => panic!("{}: {e}", p.display()),
        }
    }
    eprintln!("{ok}/{total} player streams round trip");
    assert_eq!(ok, total);
}

#[test]
fn level_data_follows_the_games_invariants() {
    let Some(files) = fixtures() else { return };
    let saves = readable(&files);
    let mut checked = 0;
    for (p, _, c) in &saves {
        let l = &c.level;
        // Past the table every level costs row 0's requiredExp (the script's GetLevelDefinition); NG+ saves sit there.
        checked += 1;
        // `GainLevel`: the experience pool's `used` is the level's requiredTotalExp.
        assert_eq!(Some(l.xp.used), l.exp_for_level(l.level), "{}", p.display());
        // `AddPoints` -> `AddAbility(GetLevelupAbility(level))`: one Lvl<N> ability per level 2..=level.
        let mut ab = c.level_abilities.clone();
        ab.sort_unstable();
        ab.dedup();
        assert_eq!(ab, (2..=l.level.min(l.last_custom_level)).collect::<Vec<_>>(), "{}", p.display());
    }
    eprintln!("{checked} saves checked ({} readable)", saves.len());
    assert!(checked > 0 || files.is_empty());
}

#[test]
fn base_vitality_is_3500_plus_the_level_abilities_on_most_saves() {
    let Some(files) = fixtures() else { return };
    let (mut n, mut ok) = (0, 0);
    for (_, _, c) in readable(&files) {
        if c.level.level > c.level.last_custom_level {
            continue;
        }
        let sum: f32 = (2..=c.level.level).filter_map(character::lvl_vitality).sum();
        if let Some(v) = c.vitality() {
            n += 1;
            ok += ((v.max - sum - 3500.0).abs() < 0.5) as i32;
        }
    }
    eprintln!("vitality max = 3500 + sum(Lvl) on {ok} of {n}");
    // Armour / skills add their own vitality on the rest (31 of 40 on C:\w3-fixtures alone, 122 of 131 with the Nexus packs).
    assert!(n == 0 || ok * 10 >= n * 7);
}

/// The game's own level-ups between two saves of one playthrough: our edit must produce the same
/// ability list, the same change of the skill-point total and the same change of the stored maximum
/// vitality.
#[test]
fn set_level_reproduces_real_level_ups() {
    let Some(files) = fixtures() else { return };
    let docs: Vec<_> = readable(&files).into_iter().filter(|(_, _, c)| c.level.level < c.level.last_custom_level).collect();
    let stem_id = |p: &Path| p.file_name().and_then(|s| s.to_str()).and_then(|s| s.split('_').nth(1)).map(str::to_owned);
    let mut by_dir: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (i, (p, _, _)) in docs.iter().enumerate() {
        by_dir.entry(p.parent().unwrap().to_path_buf()).or_default().push(i);
    }
    let (mut pairs, mut abil, mut vit, mut sp) = (0, 0, 0, 0);
    for ix in by_dir.values() {
        for &i in ix {
            for &j in ix {
                let (a, b) = (&docs[i].2, &docs[j].2);
                if b.level.level <= a.level.level || b.level.level > a.level.level + 6 || b.level.total_xp() <= a.level.total_xp() || stem_id(&docs[i].0) != stem_id(&docs[j].0) {
                    continue;
                }
                pairs += 1;
                let mut d = docs[i].1.clone();
                let chg = character::set_level(&mut d, b.level.level).unwrap();
                let r = character::read(&d).unwrap();
                let (mut x, mut y) = (r.level_abilities.clone(), b.level_abilities.clone());
                x.sort_unstable();
                y.sort_unstable();
                abil += (x == y) as i32;
                vit += ((b.vitality().unwrap().max - a.vitality().unwrap().max - chg.vitality_delta).abs() < 0.5) as i32;
                let total = |c: &Character| c.level.skill.free + c.level.skill.used;
                sp += (total(b) - total(a) == total(&r) - total(a)) as i32;
                assert_eq!(r.level.xp.used, b.level.xp.used, "xp threshold of the new level");
            }
        }
    }
    eprintln!("{pairs} real level-up pairs: abilities {abil}, vitality {vit}, skill points {sp}");
    assert_eq!(abil, pairs);
    assert_eq!(sp, pairs);
    // Gear can move the maximum between two saves; allow one miss.
    assert!(vit + 1 >= pairs);
}

#[test]
fn an_edited_save_round_trips_through_container_and_rebase() {
    let Some(files) = fixtures() else { return };
    let Some((p, d, c)) = readable(&files).into_iter().find(|(_, _, c)| c.level.level < 20 && c.level.last_custom_level == 50) else { return };
    let before = c.level.clone();
    let mut doc = d.clone();
    let chg = character::set_level(&mut doc, 20).unwrap();
    assert_eq!((chg.from, chg.to), (before.level, 20));
    let bytes = doc.to_bytes();
    // The recomputed table equals what is on disk, the payload re-parses and re-serializes identically.
    let again = Document::parse(&bytes, doc.base).unwrap();
    assert_eq!(again.to_bytes(), bytes, "{}", p.display());
    assert_eq!(again.table(), payload::read_map(&bytes, doc.base).unwrap().variables);
    let r = character::read(&again).unwrap();
    assert_eq!(r.level.level, 20);
    assert_eq!(Some(r.level.xp.used), r.level.exp_for_level(20));
    assert_eq!(r.level.xp.free, 0);
    assert_eq!(r.level_abilities.iter().copied().max(), Some(20));
    // Skill points: the table's grant for the extra levels, spent points unchanged.
    assert_eq!(r.level.skill.used, before.skill.used);
    assert_eq!(
        r.level.skill.free + r.level.skill.used - (before.skill.free + before.skill.used),
        before.skill_points_up_to(20).unwrap() - before.skill_points_up_to(before.level).unwrap()
    );
    // Items are untouched.
    assert_eq!(r.items, c.items);
    // Going back down restores the level data the template had (abilities, points of the level).
    let mut back = again.clone();
    character::set_level(&mut back, before.level).unwrap();
    let rb = character::read(&back).unwrap();
    let (mut x, mut y) = (rb.level_abilities.clone(), c.level_abilities.clone());
    x.sort_unstable();
    y.sort_unstable();
    assert_eq!(x, y);
    assert_eq!(rb.vitality().unwrap().max, c.vitality().unwrap().max);
    assert_eq!(rb.level.skill, before.skill);
}

#[test]
fn crowns_can_be_set_and_nothing_else_moves() {
    let Some(files) = fixtures() else { return };
    let mut done = 0;
    for (p, d, c) in readable(&files) {
        let Some(old) = c.crowns() else { continue };
        let mut doc = d.clone();
        if character::set_crowns(&mut doc, 1234).is_err() {
            continue;
        }
        let r = character::read(&doc).unwrap();
        assert_eq!(r.crowns(), Some(1234), "{}", p.display());
        assert_eq!(r.level, c.level);
        assert_eq!(r.items.len(), c.items.len());
        for (x, y) in c.items.iter().zip(&r.items) {
            if x.name != "Crowns" {
                assert_eq!((x.quantity, x.durability.to_bits()), (y.quantity, y.durability.to_bits()), "{}", x.name);
            }
        }
        // Restoring the old amount gives back the exact payload.
        character::set_crowns(&mut doc, old).unwrap();
        assert_eq!(doc.to_bytes(), d.to_bytes(), "{}", p.display());
        done += 1;
        if done == 12 {
            break;
        }
    }
    eprintln!("crowns edited and restored on {done} saves");
}

#[test]
fn refusals_leave_the_document_untouched() {
    let Some(files) = fixtures() else { return };
    let Some((_, d, _)) = readable(&files).into_iter().find(|(_, _, c)| c.crowns().is_some() && c.level.last_custom_level == 50) else { return };
    let bytes = d.to_bytes();
    let mut doc = d.clone();
    // Above 65535 the quantity field is widened (flag 0x2000, as the game writes it for big purses) and reads back.
    let mut wide = d.clone();
    character::set_crowns(&mut wide, 70_000).unwrap();
    assert_eq!(character::read(&wide).unwrap().crowns(), Some(70_000));
    let again = Document::parse(&wide.to_bytes(), wide.base).unwrap();
    assert_eq!(character::read(&again).unwrap().crowns(), Some(70_000));
    assert!(character::set_level(&mut doc, 0).is_err());
    assert!(character::set_level(&mut doc, 51).is_err());
    assert!(character::set_item_quantity(&mut doc, "No such item", 1).is_err());
    assert_eq!(doc.to_bytes(), bytes);
}

fn raw_records(d: &Document) -> Option<(Vec<u8>, Vec<character::RecordSpan>)> {
    let s = character::player_stream(d).ok()?;
    let (_, spans) = character::parse_records(&s.tail, s.version).ok()?;
    Some((s.tail, spans))
}

/// Acceptance: for several items the record we build is byte for byte a record the game wrote.
#[test]
fn built_records_equal_the_records_the_game_wrote() {
    use character::{ItemForm, NewItem};
    let Some(files) = fixtures() else { return };
    let specs: Vec<(&str, ItemForm)> = vec![
        ("Dol Blathanna longsword", ItemForm::Gear { weapon: true, durability: 100.0, slots: 1 }),
        ("Silver Vixen", ItemForm::Gear { weapon: true, durability: 100.0, slots: 1 }),
        ("Dol Blathanna Armor", ItemForm::Gear { weapon: false, durability: 100.0, slots: 1 }),
        ("Dol Blathanna Boots", ItemForm::Gear { weapon: false, durability: 100.0, slots: 0 }),
        ("Cat 1", ItemForm::Charged { charges: 3 }),
        ("Beast Oil 1", ItemForm::Charged { charges: 20 }),
        ("Grapeshot 1", ItemForm::Charged { charges: 2 }),
        ("Blunt Bolt", ItemForm::Stack),
    ];
    let docs: Vec<Document> = files.iter().filter_map(|p| load(p)).collect();
    let mut matched = vec![0usize; specs.len()];
    let mut seen = vec![0usize; specs.len()];
    for d in &docs {
        let Some((tail, spans)) = raw_records(d) else { continue };
        for (k, (name, form)) in specs.iter().enumerate() {
            let Some(ix) = d.name_index(name) else { continue };
            for r in spans.iter().filter(|r| r.name == ix) {
                let raw = &tail[r.start..r.end];
                let qty = u16::from_le_bytes([raw[17], raw[18]]);
                let hash = u16::from_le_bytes([raw[2], raw[3]]);
                let item = NewItem { name: (*name).to_owned(), quantity: qty, form: *form };
                let mut res = |n: &str| d.name_index(n).ok_or_else(|| w3save::Error::Unverified(format!("no name {n}")));
                // Only records of the shape add_item creates (fresh, not equipped, not looted, no enhancement or
                // ability list: same flag word as the built record) are compared; equipped / looted / worn copies of
                // the same item are other shapes and are not what add_item writes.
                let Ok(built) = character::build_record(&item, r.uid, hash, true, &mut res) else { continue };
                if built[4..6] != raw[4..6] || built.len() != raw.len() {
                    continue;
                }
                seen[k] += 1;
                if built == raw {
                    matched[k] += 1;
                }
            }
        }
    }
    for (k, (name, _)) in specs.iter().enumerate() {
        eprintln!("{name}: built record equals the game's on {} of {} records", matched[k], seen[k]);
        assert!(seen[k] == 0 || matched[k] > 0, "{name}: none of {} records reproduced", seen[k]);
    }
    let kinds = seen.iter().filter(|s| **s > 0).count();
    eprintln!("{kinds} of {} items occur in the fixtures", specs.len());
    // The Nexus packs (W3AL_EXTRA_FIXTURES) hold the gear; C:w3-fixtures alone holds only a few of them.
    let need = if std::env::var_os("W3AL_EXTRA_FIXTURES").is_some() { 5 } else { 1 };
    assert!(kinds >= need || files.is_empty(), "too few of the items occur in the fixtures");
}

#[test]
fn add_item_appends_a_record_and_keeps_the_list_consistent() {
    use character::{ItemForm, NewItem};
    let Some(files) = fixtures() else { return };
    let mut done = 0;
    for p in &files {
        let Some(d) = load(p) else { continue };
        let Some((tail, spans)) = raw_records(&d) else { continue };
        let ver = character::player_stream(&d).unwrap().version;
        let (h0, _) = character::parse_records(&tail, ver).unwrap();
        let item = NewItem { name: "Silver Vixen".into(), quantity: 1, form: ItemForm::Gear { weapon: true, durability: 100.0, slots: 1 } };
        let mut doc = d.clone();
        let Ok(uid) = character::add_item(&mut doc, &item) else { continue };
        let bytes = doc.to_bytes();
        let again = Document::parse(&bytes, doc.base).unwrap();
        assert_eq!(again.table(), payload::read_map(&bytes, doc.base).unwrap().variables);
        let (tail2, spans2) = raw_records(&again).unwrap();
        let (h1, _) = character::parse_records(&tail2, ver).unwrap();
        assert_eq!(spans2.len(), spans.len() + 1);
        assert_eq!((h1.count, h1.next_uid, uid), (h0.count + 1, h0.next_uid + 1, h0.next_uid + 1));
        assert_eq!(spans2.last().unwrap().uid, uid);
        // The old records are untouched (same bytes in the same order).
        let (a, b) = (&tail[13..spans.last().unwrap().end], &tail2[13..spans.last().unwrap().end]);
        assert_eq!(a, b);
        let r = character::read(&again).unwrap();
        assert!(r.items.iter().any(|i| i.name == "Silver Vixen" && i.quantity == 1 && i.durability == 100.0));
        // Adding it twice is refused.
        assert!(character::add_item(&mut doc, &item).is_err());
        done += 1;
        if done == 6 {
            break;
        }
    }
    eprintln!("add_item on {done} saves");
    assert!(done > 0 || files.is_empty());
}

/// Every fixture's item list tiles exactly: header, records (back to back), trailer. Also checks that the
/// records re-concatenate to the tail byte for byte and that `Character::items` agrees with the header.
#[test]
fn the_item_list_tiles_on_every_fixture() {
    let Some(files) = fixtures() else { return };
    let (mut total, mut tiled) = (0, 0);
    for p in &files {
        let Some(d) = load(p) else { continue };
        let Ok(s) = character::player_stream(&d) else { continue };
        total += 1;
        match character::parse_records(&s.tail, s.version) {
            Ok((h, spans)) => {
                assert_eq!(spans.len(), h.count as usize, "{}", p.display());
                let mut cat = s.tail[..13].to_vec();
                for r in &spans {
                    cat.extend_from_slice(&s.tail[r.start..r.end]);
                }
                cat.extend_from_slice(&s.tail[s.tail.len() - 6..]);
                assert_eq!(cat, s.tail, "{}", p.display());
                assert_eq!(character::read(&d).unwrap().items.len(), spans.len());
                tiled += 1;
            }
            Err(e) => panic!("{}: {e}", p.display()),
        }
    }
    eprintln!("item list tiles on {tiled} of {total} saves");
    assert_eq!(tiled, total);
}
