//! Fixture-gated: the start kit on real saves. Skipped (passes) when `W3AL_FIXTURES` is unset.
use std::path::{Path, PathBuf};

use w3al_core::start_kit::{self, Gear, StartKit};
use w3al_core::Run;
use w3save::character;
use w3save::{Container, Document};

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

#[test]
fn start_kit_applies_and_verifies_on_real_saves() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else { return };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    let kit = StartKit { level: Some(25), crowns: Some(1500), gear: Gear::Recommended, ..StartKit::default() };
    let mut done = 0;
    for p in files {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let Ok(c) = Container::read(&bytes) else { continue };
        let Ok(d) = Document::parse(&c.payload, c.data_start) else { continue };
        let Ok(ch) = character::read(&d) else { continue };
        if ch.crowns().is_none() || ch.level.last_custom_level != 50 || ch.level.skill.used > 20 {
            continue;
        }
        let (out, report) = match start_kit::apply_to_save(&bytes, &kit) {
            Ok(x) => x,
            // A save whose item list has record shapes we do not understand is refused, not guessed.
            Err(e) if e.contains("not yet verified") => continue,
            Err(e) => panic!("{}: {e}", p.display()),
        };
        assert!(!report.is_empty());
        let c2 = Container::read(&out).unwrap();
        let r = character::read(&Document::parse(&c2.payload, c2.data_start).unwrap()).unwrap();
        assert_eq!((r.level.level, r.crowns()), (25, Some(1500)));
        done += 1;
        if done == 6 {
            break;
        }
    }
    eprintln!("start kit applied on {done} saves");
}

/// `new_game_plus = false` through the whole start-kit path (apply, re-read, verify) on the New Game+ saves
/// among the fixtures, alone and together with a level change; a save that is not a New Game+ is refused nothing
/// and changes nothing.
#[test]
fn new_game_plus_off_applies_and_verifies_on_real_saves() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else { return };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    let off = StartKit { new_game_plus: Some(false), ..StartKit::default() };
    let both = StartKit { level: Some(20), new_game_plus: Some(false), ..StartKit::default() };
    let (mut done, mut plain) = (0, 0);
    for p in files {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let Ok(c) = Container::read(&bytes) else { continue };
        let Ok(d) = Document::parse(&c.payload, c.data_start) else { continue };
        let Ok(before) = w3save::newgameplus::read(&d) else { continue };
        if character::read(&d).map_or(true, |ch| ch.level.last_custom_level != 50 || ch.level.skill.used > 40) {
            continue;
        }
        if !before.is_new_game_plus() && plain >= 4 {
            continue;
        }
        for kit in [&off, &both] {
            let (out, report) = match start_kit::apply_to_save(&bytes, kit) {
                Ok(x) => x,
                Err(e) if e.contains("not yet verified") => continue,
                Err(e) => panic!("{}: {e}", p.display()),
            };
            let c2 = Container::read(&out).unwrap();
            let after = w3save::newgameplus::read(&Document::parse(&c2.payload, c2.data_start).unwrap()).unwrap();
            assert!(after.is_clean(), "{}: {after:?}", p.display());
            assert!(!report.is_empty());
            if before.is_new_game_plus() {
                done += 1;
            } else {
                plain += 1;
            }
        }
        if done >= 8 {
            break;
        }
    }
    eprintln!("New Game+ off applied on {done} New Game+ and {plain} other saves");
}

/// `skill_points` on real saves: alone and after a level change, the total is exactly what was asked, spent
/// points stay, and a total below the spent points is refused.
#[test]
fn skill_points_apply_and_verify_on_real_saves() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else { return };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    let mut done = 0;
    for p in files {
        let Ok(bytes) = std::fs::read(&p) else { continue };
        let Ok(c) = Container::read(&bytes) else { continue };
        let Ok(d) = Document::parse(&c.payload, c.data_start) else { continue };
        let Ok(ch) = character::read(&d) else { continue };
        if ch.level.last_custom_level != 50 || ch.level.skill.used > 17 {
            continue;
        }
        let used = ch.level.skill.used as u32;
        for kit in [StartKit { skill_points: Some(used + 3), ..StartKit::default() }, StartKit { level: Some(18), skill_points: Some(17.max(used)), ..StartKit::default() }] {
            let (out, report) = match start_kit::apply_to_save(&bytes, &kit) {
                Ok(x) => x,
                Err(e) if e.contains("not yet verified") => continue,
                Err(e) => panic!("{}: {e}", p.display()),
            };
            assert!(report.iter().any(|l| l.contains("ability points")), "{report:?}");
            let c2 = Container::read(&out).unwrap();
            let r = character::read(&Document::parse(&c2.payload, c2.data_start).unwrap()).unwrap();
            assert_eq!((r.level.skill.free + r.level.skill.used) as u32, kit.skill_points.unwrap(), "{}", p.display());
            assert_eq!(r.level.skill.used, ch.level.skill.used);
            done += 1;
        }
        if used > 0 {
            let low = StartKit { skill_points: Some(used - 1), ..StartKit::default() };
            assert!(start_kit::apply_to_save(&bytes, &low).is_err(), "{}: below spent must be refused", p.display());
        }
        if done >= 8 {
            break;
        }
    }
    eprintln!("ability points set on {done} saves");
}

#[test]
fn new_game_plus_on_is_refused() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else { return };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    files.sort();
    let Some(p) = files.first() else { return };
    let bytes = std::fs::read(p).unwrap();
    let kit = StartKit { new_game_plus: Some(true), ..StartKit::default() };
    assert!(start_kit::apply_to_save(&bytes, &kit).is_err());
}

#[test]
fn run_file_carries_the_section() {
    let run: Run = toml::from_str("[start_kit]\nlevel = 12\ncrowns = 800\ngear = \"recommended\"\n").unwrap();
    assert_eq!(run.start_kit, Some(StartKit { level: Some(12), crowns: Some(800), gear: Gear::Recommended, ..StartKit::default() }));
    let sp: Run = toml::from_str("[start_kit]\nskill_points = 20\n").unwrap();
    assert_eq!(sp.start_kit.and_then(|k| k.skill_points), Some(20));
    let back: Run = toml::from_str(&toml::to_string_pretty(&run).unwrap()).unwrap();
    assert_eq!(back, run);
    assert!(toml::from_str::<Run>("").unwrap().start_kit.is_none());
}
