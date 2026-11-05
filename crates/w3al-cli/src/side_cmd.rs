//! Side quests: `w3al res-dump` (a side resource's exports), and the commands built on
//! `w3graph::complete` (auto-complete walk) and `w3graph::catalog` (the quest list of the install).
use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{anyhow, Result};
use w3graph::catalog::{self, JournalQuest};
use w3graph::complete;
use w3graph::cr2w;
use w3graph::frontier;
use w3graph::quest::{guid_hex, short, BlockKind, Library};
use w3graph::synth;
use w3graph::walk::Event;
use w3save::journal;

/// `w3al res-dump`: every export of side resources (`.journal`, `.w2comm`, `.w2scene`) whose name contains `name`,
/// with its properties (research tool; prints game identifiers only to the terminal).
pub fn res_dump_cmd(install: &Path, name: &str, max: usize) -> Result<ExitCode> {
    let lib = Library::load_install(install).map_err(|e| anyhow!("{e}"))?;
    let needle = name.to_ascii_lowercase().replace('\\', "/");
    let mut n = 0;
    for ext in [".journal", ".w2comm", ".w2scene"] {
        for r in lib.resource_names(ext).into_iter().filter(|r| r.contains(&needle)) {
            n += 1;
            if max == 0 {
                println!("{r}");
                continue;
            }
            if n > max {
                println!("... more resources match; narrow the name");
                return Ok(ExitCode::SUCCESS);
            }
            let d = lib.read_resource(&r).map_err(|e| anyhow!("{e}"))?;
            let c = cr2w::parse(&d, &r).map_err(|e| anyhow!("{e}"))?;
            println!("== {r}: {} exports", c.exports.len());
            for (i, e) in c.exports.iter().enumerate() {
                println!("  #{i} {} parent={:?}", e.class, e.parent);
                for p in &e.props {
                    println!("      {}: {} = {}", p.name, p.ty, p.value);
                }
            }
        }
    }
    if n == 0 {
        println!("no side resource matches {name:?}");
    }
    Ok(ExitCode::SUCCESS)
}

fn print_completion(lib: &Library, c: &complete::Completion, verbose: bool) {
    println!("   instances: {}", c.instances.iter().map(|i| lib.describe(*i)).collect::<Vec<_>>().join("; "));
    println!("   starts ({}): {}", c.starts.len(), c.starts.iter().take(6).map(|(b, i)| format!("{}<-{i:?}", lib.blocks[*b].short())).collect::<Vec<_>>().join(" "));
    for p in &c.picks {
        println!("   pick {}={}  ({}; {})", lib.blocks[p.block].short(), p.socket, p.why, lib.describe(p.block));
    }
    println!("   steps {}, parked {}, facts [{}], removed [{}]", c.walk.steps, c.walk.auto_parked.len(), c.walk.facts.iter().filter(|(_, v)| **v > 0).map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "), c.walk.removed_facts.join(" "));
    let j: Vec<String> = c
        .walk
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Journal { path, status, .. } => Some(format!("{}:{}", path.iter().map(|s| short(&s.guid)).collect::<Vec<_>>().join("/"), status.trim_start_matches("JS_"))),
            _ => None,
        })
        .collect();
    println!("   journal {}", j.join(" "));
    println!("   exited {} scope(s), {} new wait(s) outside", c.walk.exited.len(), c.walk.waits.len());
    if verbose {
        for w in &c.walk.waits {
            println!("     wait {}", lib.describe(w.block));
        }
        for e in &c.walk.events {
            if let Event::Assumed { block, socket, why } = e {
                println!("     assumed {} -> {socket:?} ({why})", lib.describe(*block));
            }
        }
    }
}

/// The decision files refused by the completion walk: the catalogue's plus every file of a block the
/// questbook's own decision effects name (synth start / pick / stop / scene pick).
pub fn deny_files(lib: &Library, qb: Option<&Path>) -> BTreeSet<String> {
    let mut v: BTreeSet<String> = complete::DECISION_FILES.iter().map(|s| (*s).to_owned()).collect();
    if let Some(dir) = w3al_core::paths::locate_questbook(qb.map(Path::to_path_buf)) {
        if let Ok(book) = w3al_core::Questbook::load_dir(&dir) {
            for f in w3al_core::side::decision_files(&book, lib) {
                v.insert(f);
            }
        }
    }
    v
}

/// `w3al side-complete`: auto-complete quests on a template (each from the result of the previous), print the
/// walk, optionally write the save.
pub fn side_complete_cmd(install: &Path, template: &Path, keys: &[String], out_dir: Option<&Path>, verbose: bool) -> Result<ExitCode> {
    let lib = Library::load_install(install).map_err(|e| anyhow!("{e}"))?;
    let idx = complete::QuestIndex::new(&lib).map_err(|e| anyhow!("{e}"))?;
    let opts = complete::Options { deny_files: deny_files(&lib, None).into_iter().collect(), ..Default::default() };
    let mut doc = crate::graph_cmd::load_doc(template)?;
    let mut done = 0;
    for key in keys {
        let Some(q) = idx.find(key).or_else(|| find_quests(&idx.quests, key).first().copied()) else {
            println!("== {key}: no such quest");
            continue;
        };
        println!("== {} {} ({})", guid_hex(&q.guid), q.base_name, q.file);
        let tf = frontier::read(&doc).map_err(|e| anyhow!("{e}"))?;
        match complete::complete(&lib, &idx, &doc, &tf, q, &opts) {
            Err(r) => println!("   REFUSED {r}"),
            Ok(c) => {
                print_completion(&lib, &c, verbose);
                match synth::apply(&lib, &c.walk, &doc) {
                    Ok((d, r)) => {
                        println!("   written: blocks -{} +{}, scopes -{} +{}, journal {} set {} appended, facts {}", r.removed_blocks.len(), r.added_blocks.len(), r.removed_scopes.len(), r.added_scopes.len(), r.journal.status_set, r.journal.appended, r.facts_set.len());
                        doc = d;
                        done += 1;
                    }
                    Err(e) => println!("   WRITE FAILED {e}"),
                }
            }
        }
    }
    if let Some(out) = out_dir {
        if done == 0 {
            println!("nothing completed; nothing written");
            return Ok(ExitCode::FAILURE);
        }
        let bytes = std::fs::read(template)?;
        let mut container = w3save::Container::read(&bytes)?;
        container.payload = doc.to_bytes();
        let b = container.write()?;
        let stem = w3al_core::export::default_stem(template, out);
        let r = w3al_core::export::write_save_set_as(template, out, &stem, &b)?;
        println!("wrote {}", r.sav.display());
    }
    Ok(ExitCode::SUCCESS)
}

/// `w3al side-check A B`: every quest of the install that is not done in A and done (Success) in B is
/// auto-completed on A alone and compared with B (journal of the quest, its facts, its phase, spawn sets).
pub fn side_check_cmd(install: &Path, a: &Path, b: &Path, grep: Option<&str>, verbose: bool) -> Result<ExitCode> {
    let lib = Library::load_install(install).map_err(|e| anyhow!("{e}"))?;
    let idx = complete::QuestIndex::new(&lib).map_err(|e| anyhow!("{e}"))?;
    let opts = complete::Options { deny_files: deny_files(&lib, None).into_iter().collect(), ..Default::default() };
    let (da, db) = (crate::graph_cmd::load_doc(a)?, crate::graph_cmd::load_doc(b)?);
    let tf = frontier::read(&da).map_err(|e| anyhow!("{e}"))?;
    let (ja, jb) = (journal::read(&da)?, journal::read(&db)?);
    let st = |j: &journal::Journal, q: &JournalQuest| j.find(&q.root_key()).first().map(|i| j.entries[*i].status.clone());
    let (mut hits, mut misses, mut refused) = (0, 0, 0);
    for q in &idx.quests {
        if grep.is_some_and(|g| !q.file.contains(g) && !q.base_name.to_ascii_lowercase().contains(&g.to_ascii_lowercase())) {
            continue;
        }
        let (sa, sb) = (st(&ja, q), st(&jb, q));
        if sb.as_deref() != Some("JS_Success") || matches!(sa.as_deref(), Some("JS_Success" | "JS_Failed")) {
            continue;
        }
        let name = q.file.rsplit('/').next().unwrap_or(&q.file).trim_end_matches(".journal");
        match complete::complete(&lib, &idx, &da, &tf, q, &opts) {
            Err(r) => {
                refused += 1;
                println!("REFUSED {name:<34} {} {:?} [{}]  {r}", q.qtype, sa, short(&q.guid));
            }
            Ok(c) => match synth::apply(&lib, &c.walk, &da) {
                Err(e) => {
                    refused += 1;
                    println!("REFUSED {name:<34} write: {e}");
                }
                Ok((sd, _)) => {
                    let k = complete::check(&lib, q, &c, &da, &sd, &db).map_err(|e| anyhow!("{e}"))?;
                    let tag = if k.hit() { hits += 1; "HIT    " } else { misses += 1; "MISS   " };
                    println!(
                        "{tag} {name:<34} {} {:?} picks {} journal {}/{}/{} facts {}/{}/{} phase {}/{} sets {}/{}",
                        q.qtype, sa, c.picks.len(), k.journal.0, k.journal.1, k.journal.2, k.facts.0, k.facts.1, k.facts.2, k.scope_ended.0, k.scope_ended.1, k.community.0, k.community.1
                    );
                    if verbose || !k.hit() {
                        for l in k.lines.iter().take(if verbose { 200 } else { 8 }) {
                            println!("        {l}");
                        }
                    }
                    if verbose {
                        print_completion(&lib, &c, false);
                    }
                }
            },
        }
    }
    println!("# {hits} hit, {misses} miss, {refused} refused");
    Ok(ExitCode::SUCCESS)
}

/// Graph files every questbook synth effect acts in (start / pick / stop / scene pick blocks; hubs aside): a quest
/// living there is the decision questbook's (or its preparatory walk's), not a side quest.
fn covered_files(book: &w3al_core::Questbook, lib: &Library) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for q in &book.quests {
        for e in q.effects.values() {
            let Some(s) = &e.synth else { continue };
            let keys = s.start.iter().map(|k| k.split(':').next().unwrap_or(k)).chain(s.pick.iter().chain(&s.scene_pick).map(|k| k.split('=').next().unwrap_or(k))).chain(s.stop.iter().map(String::as_str));
            for k in keys {
                let hits: Vec<usize> = if k.len() == 32 { w3graph::quest::parse_guid(k).ok().and_then(|g| lib.get(&g)).into_iter().collect() } else { lib.by_short(k).to_vec() };
                if let [b] = hits[..] {
                    let f = &lib.blocks[b].file;
                    if !complete::is_shared_file(f) && !f.ends_with(".w2quest") && !f.contains("sidequests") {
                        out.insert(complete::file_name(f));
                    }
                }
            }
        }
    }
    out
}

/// Quests the generator never lists: the expansions (out of v1), demo, test and debug quests.
fn excluded(q: &JournalQuest) -> Option<&'static str> {
    let f = q.file.to_ascii_lowercase();
    let g = q.group_name.to_ascii_lowercase();
    if f.starts_with("dlc/ep1/") || f.starts_with("dlc/bob/") || g.starts_with("bob") || g.contains("ep1") {
        return Some("expansion (out of v1)");
    }
    let stem = f.rsplit('/').next().unwrap_or(&f);
    if ["demo", "debug", "test", "e3", "tipstricks", "tutorial", "template"].iter().any(|s| stem.contains(s)) || ["demo quests", "testlevel", "temp minortestingquest"].contains(&g.as_str()) {
        return Some("demo / test");
    }
    None
}

pub struct GenArgs<'a> {
    pub install: &'a Path,
    pub out: &'a Path,
    pub template_dirs: &'a [std::path::PathBuf],
    pub questbook: Option<std::path::PathBuf>,
    pub dry_run: bool,
}

/// `w3al gen-side-quests`: one TOML per side quest (and main quest between decisions) that the auto-complete
/// walk finishes on at least one registered window template; totals per template and refusals by reason.
pub fn gen_side_quests_cmd(a: &GenArgs) -> Result<ExitCode> {
    use std::collections::BTreeMap;
    use w3al_core::side::{classify, side_id, SideKind};
    let lib = Library::load_install(a.install).map_err(|e| anyhow!("{e}"))?;
    let idx = complete::index(&lib).map_err(|e| anyhow!("{e}"))?;
    let qb = w3al_core::paths::locate_questbook(a.questbook.clone()).ok_or_else(|| anyhow!("no questbook folder found"))?;
    let book = w3al_core::Questbook::load_dir(&qb)?;
    let reg = w3al_core::templates::Registry::load_for_questbook(&qb).map_err(|e| anyhow!(e))?;
    let deny = deny_files(&lib, Some(&qb));
    let covered = covered_files(&book, &lib);
    let opts = complete::Options { deny_files: deny.iter().cloned().collect(), ..Default::default() };
    // Templates in line order (those whose file is found).
    let mut temps: Vec<(String, u32, w3save::Document, frontier::Frontier)> = Vec::new();
    let mut ts: Vec<&w3al_core::templates::Template> = reg.templates.iter().collect();
    ts.sort_by_key(|t| t.line);
    for t in ts {
        let Some(p) = w3al_core::templates::find_file(a.template_dirs, &t.stem) else {
            eprintln!("template {} ({}.sav) not found; skipped", t.id, t.stem);
            continue;
        };
        let d = crate::graph_cmd::load_doc(&p)?;
        let f = frontier::read(&d).map_err(|e| anyhow!("{e}"))?;
        temps.push((t.id.clone(), t.line, d, f));
    }
    eprintln!("{} quests in the install, {} templates", idx.quests.len(), temps.len());
    // The game's recommended levels (`gameplay\globals\quest_levels.csv` and the DLC tables), by journal GUID.
    let levels: std::collections::HashMap<String, u32> = w3graph::names::quest_levels(a.install)
        .map_err(|e| anyhow!("{e}"))?
        .into_iter()
        .filter_map(|l| Some((l.guid, l.level?)))
        .collect();

    struct Row {
        q: JournalQuest,
        home: String,
        kind: SideKind,
        region: &'static str,
        act: u8,
        outcomes: BTreeMap<String, String>,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
    let mut refused: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Per template: outcome label -> count.
    let mut per_template: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut quests: Vec<&JournalQuest> = idx.quests.iter().collect();
    quests.sort_by(|x, y| x.file.cmp(&y.file));
    for q in quests {
        let name = q.file.rsplit('/').next().unwrap_or(&q.file).trim_end_matches(".journal").to_owned();
        if let Some(why) = excluded(q) {
            *skipped.entry(why.to_owned()).or_default() += 1;
            continue;
        }
        let Some(home) = complete::home(&lib, &idx, q) else {
            refused.entry("no quest graph (journal written by scripts: notes, points of interest)".into()).or_default().push(name);
            continue;
        };
        let (home_file, insts) = match &home {
            complete::Home::File(f) => (f.clone(), lib.importers_of(f).to_vec()),
            complete::Home::Phase(p) => (lib.blocks[*p].file.clone(), vec![*p]),
        };
        if covered.contains(&complete::file_name(&home_file)) {
            *skipped.entry("covered by the decision questbook".into()).or_default() += 1;
            continue;
        }
        if let Some(f) = complete::region(&lib, &insts).iter().map(|b| complete::file_name(&lib.blocks[*b].file)).find(|f| deny.contains(f)) {
            refused.entry("decision quest (catalogue / decision questbook)".into()).or_default().push(format!("{name} ({f})"));
            continue;
        }
        let c = classify(&q.qtype, &q.group_name, &q.file, &home_file, q.world);
        let mut outcomes = BTreeMap::new();
        for (id, _, d, f) in &temps {
            let o = match complete::complete(&lib, &idx, d, f, q, &opts) {
                Ok(done) => match synth::apply(&lib, &done.walk, d) {
                    Ok(_) => "ok".to_owned(),
                    Err(e) => format!("write failed: {e}"),
                },
                Err(r) if r.kind == complete::Refusal::AlreadyDone => "done".to_owned(),
                Err(r) => r.kind.label().to_owned(),
            };
            let key = if o.starts_with("write failed") { "write failed".to_owned() } else { o.clone() };
            *per_template.entry(id.clone()).or_default().entry(key).or_default() += 1;
            outcomes.insert(id.clone(), o);
        }
        if outcomes.values().any(|o| o == "ok") {
            rows.push(Row { q: q.clone(), home: home_file, kind: c.kind, region: c.region, act: c.act, outcomes });
        } else {
            let why = outcomes.values().find(|o| *o != "done").cloned().unwrap_or_else(|| "done on every template".into());
            refused.entry(why).or_default().push(name);
        }
    }

    // Orders: act, kind, journal file.
    let rank = |k: SideKind| match k {
        SideKind::Main => 0,
        SideKind::Side => 1,
        SideKind::Contract => 2,
        SideKind::Treasure => 3,
        SideKind::Scavenger => 4,
        SideKind::Gwent => 5,
        SideKind::Race => 6,
        SideKind::Fistfight => 7,
    };
    rows.sort_by(|x, y| (x.act, rank(x.kind), &x.q.file).cmp(&(y.act, rank(y.kind), &y.q.file)));
    let mut seq: BTreeMap<(u8, u32), u32> = BTreeMap::new();
    let mut ids: BTreeSet<String> = book.quests.iter().map(|q| q.id.clone()).collect();
    let mut files: Vec<(std::path::PathBuf, String)> = Vec::new();
    for r in &rows {
        let n = seq.entry((r.act, rank(r.kind))).or_insert(0);
        *n += 1;
        let order = u32::from(r.act) * 10_000 + rank(r.kind) * 1000 + *n;
        let mut id = side_id(&r.q.file);
        while ids.contains(&id) {
            id.push_str("_2");
        }
        ids.insert(id.clone());
        let journal_key = format!("{}/{}", guid_hex(&r.q.group), guid_hex(&r.q.guid));
        let kind = serde_json::to_value(r.kind)?.as_str().unwrap_or("side").to_owned();
        let fallback = r.home.trim_end_matches(".w2phase").trim_end_matches(".w2quest").to_owned();
        let mut t = String::new();
        t.push_str("# Generated by `w3al gen-side-quests` from the quest graphs and journals of the player's own install: identifiers\n");
        t.push_str("# only, no game text (the official title is read from the install at run time by `title_id`). Re-run the\n");
        t.push_str("# generator instead of editing. Schema: questbook/SCHEMA.md \"Side quests\". Licensed CC BY-SA 4.0.\n");
        t.push_str(&format!("id = \"{id}\"\nkind = \"{kind}\"\nregion = \"{}\"\nact = {}\norder = {order}\n", r.region, r.act));
        if let Some(lv) = levels.get(&guid_hex(&r.q.guid)) {
            t.push_str(&format!("level = {lv}   # from the game's quest-level table\n"));
        }
        if let Some(tid) = r.q.title_id {
            t.push_str(&format!("title_id = {tid}\n"));
        }
        t.push_str(&format!("fallback_title = \"{fallback}\"\njournal = \"{journal_key}\"\nphase = \"{}\"\ndone_effects = \"done\"\n", r.home));
        t.push_str("\n[templates]   # the generator's trial on each window template: ok | done | the refusal\n");
        for (k, v) in &r.outcomes {
            t.push_str(&format!("{k} = \"{}\"\n", v.replace('"', "'")));
        }
        t.push_str(&format!(
            "\n[effects.done]\nsource = \"quest graph {} (auto-complete walk to the journal root's Success and the phase's end, w3graph::complete)\"\n\n[effects.done.synth]\ncomplete = \"{journal_key}\"\n",
            r.home
        ));
        let sub = if r.kind == SideKind::Main { "main".to_owned() } else { r.region.to_owned() };
        files.push((a.out.join(sub).join(format!("{id}.toml")), t));
    }

    println!("# per template (line order): outcome counts over the {} candidate quests", rows.len() + refused.values().map(Vec::len).sum::<usize>());
    for (id, _, _, _) in &temps {
        let m = per_template.get(id).cloned().unwrap_or_default();
        println!("  {id:<20} {}", m.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", "));
    }
    let mains = rows.iter().filter(|r| r.kind == SideKind::Main).count();
    println!("# written: {} side quests and {mains} main quests (complete on at least one template)", rows.len() - mains);
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for r in &rows {
        *by_kind.entry(format!("{:?}", r.kind)).or_default() += 1;
    }
    println!("  by kind: {}", by_kind.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "));
    println!("# not written:");
    for (k, v) in &skipped {
        println!("  skipped, {k}: {v}");
    }
    for (k, v) in &refused {
        println!("  refused, {k}: {} ({})", v.len(), v.iter().take(12).cloned().collect::<Vec<_>>().join(" "));
    }
    if a.dry_run {
        return Ok(ExitCode::SUCCESS);
    }
    // Replace the previous generation (only `.toml` files under the output folder).
    if a.out.is_dir() {
        for e in walk_toml(a.out) {
            std::fs::remove_file(e)?;
        }
    }
    for (p, t) in &files {
        std::fs::create_dir_all(p.parent().unwrap_or(a.out))?;
        std::fs::write(p, t)?;
    }
    println!("wrote {} files under {}", files.len(), a.out.display());
    Ok(ExitCode::SUCCESS)
}

fn walk_toml(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().is_some_and(|x| x == "toml") {
                out.push(p);
            }
        }
    }
    out
}

fn hexs(b: &[u8], n: usize) -> String {
    b.iter().take(n).map(|x| format!("{x:02x}")).collect::<String>() + if b.len() > n { ".." } else { "" }
}

fn dump_parts(doc: &w3save::Document, parts: &[w3save::token::Part], depth: usize, max_depth: usize) {
    for p in parts {
        match p {
            w3save::token::Part::Bytes(b) => println!("{}bytes[{}] {}", "  ".repeat(depth), b.len(), hexs(b, 48)),
            w3save::token::Part::Node(n) => dump_node(doc, n, depth, max_depth),
        }
    }
}

fn dump_node(doc: &w3save::Document, n: &w3save::token::Node, depth: usize, max_depth: usize) {
    use w3save::token::Kind;
    let pad = "  ".repeat(depth);
    match &n.kind {
        Kind::Raw(r) => {
            println!("{pad}RAW tag={:?} why={} parts={} len={}", r.tag, r.why, r.parts.len(), n.encoded_len());
            if depth < max_depth {
                dump_parts(doc, &r.parts, depth + 1, max_depth);
            }
        }
        Kind::Block(b) => {
            println!("{pad}BS {} ({} children)", doc.name(b.name).unwrap_or("?"), b.children.len());
            if depth < max_depth {
                for c in &b.children {
                    dump_node(doc, c, depth + 1, max_depth);
                }
            }
        }
        Kind::Var(v) => {
            println!("{pad}VAR {} = {}", doc.name(v.name).unwrap_or("?"), w3save::graph::value_text(doc, &v.value));
            if let w3save::token::Value::Opaque(parts) = &v.value {
                if depth < max_depth {
                    dump_parts(doc, parts, depth + 1, max_depth);
                }
            }
        }
        _ => println!("{pad}{:?}", n.tag()),
    }
}

/// `w3al node-dump SAV PATH`: the token structure under a path, raw parts included (research).
pub fn node_dump_cmd(sav: &Path, path: &str, depth: usize) -> Result<ExitCode> {
    let doc = crate::graph_cmd::load_doc(sav)?;
    let loc = w3save::graph::locate(&doc, path)?.ok_or_else(|| anyhow!("{path} not found"))?;
    let n = w3save::graph::node_at(&doc, &loc);
    dump_node(&doc, n, 0, depth);
    Ok(ExitCode::SUCCESS)
}

/// The catalogue entries a key names: a GUID (8 or 32 hex), or a piece of the internal name / journal file.
pub fn find_quests<'a>(quests: &'a [JournalQuest], key: &str) -> Vec<&'a JournalQuest> {
    let k = key.to_ascii_lowercase();
    quests
        .iter()
        .filter(|q| {
            let g = guid_hex(&q.guid);
            g == k || (k.len() == 8 && g.starts_with(&k)) || q.base_name.to_ascii_lowercase().contains(&k) || q.file.contains(&k)
        })
        .collect()
}

/// `w3al quest-info`: what the install says about a quest (journal entry, graph blocks that write it, their
/// files and importers) and, with a template, its state there.
pub fn quest_info_cmd(install: &Path, template: Option<&Path>, keys: &[String], brief: bool) -> Result<ExitCode> {
    let lib = Library::load_install(install).map_err(|e| anyhow!("{e}"))?;
    let quests = catalog::journal_quests(&lib).map_err(|e| anyhow!("{e}"))?;
    let by_quest = catalog::journal_blocks_by_quest(&lib);
    let tdoc = template.map(crate::graph_cmd::load_doc).transpose()?;
    let tf = tdoc.as_ref().map(frontier::read).transpose().map_err(|e| anyhow!("{e}"))?;
    let tj = tdoc.as_ref().map(journal::read).transpose()?;
    for key in keys {
        for q in find_quests(&quests, key) {
            println!("== {} {:?} ({}) type {} world {} group {:?} {}", guid_hex(&q.guid), q.base_name, q.file, q.qtype, q.world, q.group_name, short(&q.group));
            if brief {
                continue;
            }
            if let Some(p) = &q.phase_file {
                println!("   questPhase {p}");
            }
            let blocks = by_quest.get(&q.guid).cloned().unwrap_or_default();
            let mut files: BTreeSet<String> = blocks.iter().map(|b| lib.blocks[*b].file.clone()).collect();
            files.extend(q.phase_file.clone());
            for &b in &blocks {
                let BlockKind::Journal { path, .. } = &lib.blocks[b].kind else { continue };
                println!("   journal {} depth {} inputs {:?}", lib.describe(b), path.len(), lib.input_sockets(b));
            }
            for f in &files {
                let imps: Vec<String> = lib.importers_of(f).iter().map(|i| lib.describe(*i)).collect();
                println!("   file {f}: imported by {}", if imps.is_empty() { "-".into() } else { imps.join("; ") });
            }
            if let (Some(tf), Some(tj)) = (&tf, &tj) {
                let st: Vec<String> = tj.find(&q.root_key()).iter().map(|i| tj.entries[*i].status.clone()).collect();
                println!("   template: root {}", if st.is_empty() { "absent".into() } else { st.join(",") });
                for b in &tf.blocks {
                    let Some(i) = lib.get(&b.guid) else { continue };
                    let in_files = files.contains(&lib.blocks[i].file);
                    let in_scope = b.scope.iter().any(|s| lib.get(s).is_some_and(|p| matches!(&lib.blocks[p].kind, BlockKind::Phase { import: Some(f), .. } if files.contains(f))));
                    if in_files || in_scope {
                        println!("   waits {} input={:?} scope {}", lib.describe(i), b.input, b.scope.iter().map(short).collect::<Vec<_>>().join(">"));
                    }
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
