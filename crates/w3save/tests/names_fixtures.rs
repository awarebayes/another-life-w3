//! Fixture-gated: decode the NM/MANU name table of every real save under `W3AL_FIXTURES`.
//! Skipped (passes) when the variable is unset. Fixtures are copies of real saves and are
//! never committed.
use std::path::{Path, PathBuf};

use w3save::{payload, Container};

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
fn name_table_decodes_exactly_on_every_fixture() {
    let Some(dir) = std::env::var_os("W3AL_FIXTURES") else {
        eprintln!("W3AL_FIXTURES not set; skipping");
        return;
    };
    let mut files = Vec::new();
    saves(Path::new(&dir), &mut files);
    let mut failures = Vec::new();
    for f in &files {
        let result = (|| -> w3save::Result<usize> {
            let c = Container::read(&std::fs::read(f).map_err(|e| w3save::Error::Payload(e.to_string()))?)?;
            let map = payload::read_map(&c.payload, c.data_start)?;
            let names = payload::read_names(&c.payload, &map)?;
            Ok(names.len())
        })();
        match result {
            Ok(n) if n > 0 => {}
            Ok(_) => failures.push(format!("{}: empty table", f.display())),
            Err(e) => failures.push(format!("{}: {e}", f.display())),
        }
    }
    assert!(failures.is_empty(), "{} of {} saves failed:\n{}", failures.len(), files.len(), failures.join("\n"));
}
