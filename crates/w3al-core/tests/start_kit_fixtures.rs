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
    let kit = StartKit { level: Some(25), crowns: Some(1500), gear: Gear::Recommended, items: vec![] };
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

#[test]
fn run_file_carries_the_section() {
    let run: Run = toml::from_str("[start_kit]\nlevel = 12\ncrowns = 800\ngear = \"recommended\"\n").unwrap();
    assert_eq!(run.start_kit, Some(StartKit { level: Some(12), crowns: Some(800), gear: Gear::Recommended, items: vec![] }));
    let back: Run = toml::from_str(&toml::to_string_pretty(&run).unwrap()).unwrap();
    assert_eq!(back, run);
    assert!(toml::from_str::<Run>("").unwrap().start_kit.is_none());
}
