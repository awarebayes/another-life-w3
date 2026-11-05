# Scaffold decisions and assumptions

Choices I made while scaffolding, as the brief asked, so you can iterate. The plan and
brief win wherever they speak; this file only covers what they leave open.

## What is real vs. stubbed

- **Real and tested:** container read/write (`SNFHFZLC`, LZ4 chunks, writer refuses a table that
  overruns `data_start`), packed strings, bounds-checked reader, footer / variable-table / `RB`
  readers, parse-to-regions and serialize, questbook loader, lint, resolve (answers -> effects),
  run.toml, export plumbing (new file, `create_new`, sidecar copy), the six Tauri commands, the UI.
- **Hypotheses (marked `HYPOTHESIS:` in code):** the "end offset is absolute" chunk-record rule (holds
  on 51 of 56 Nexus saves), which header int is which. (The NM/MANU layout was a hypothesis until
  2026-11-04 and is now confirmed, see below; so is the fact record, which is **not** "first i32 = value": it
  is u16 value, f32 time, f32 expiry.) Each fails loudly (an error or a `check-save`
  warning), never silently.
- **Real since 2026-11-04 (late night), unproven in game:** `Document::rebase` (variable table, `RB`, pointer block,
  footer; layout PROVEN on 118 of 118 fixture files and byte-exact as a no-op, synthetic growth/shrink checked on 15 real
  saves) and `Document::apply` for facts (see "Facts, part 2" below). Still `position::{get,set}` are `todo!()`, and
  `apply` returns `Error::Unverified` for quest and journal edits (nothing is guessed about `questSystem` / `CJournalManager`;
  the separate journal editor is below, "Journal editor").
  I used `Error::Unverified` rather than `todo!()` wherever the CLI or app can reach the code,
  so `dump`, `export` and the UI report instead of panicking.
- **Synthetic payload tests prove self-consistency, not format correctness.** Only Phase 0 on a real
  5.01 save proves the layout.
- **Regions are typed trees since 2026-11-04** (`token.rs`): `Region::Variable` holds a `Node`; gaps (header, `RB`,
  pointer block, table, footer) stay raw. The round trip is now a real test (101 of 101 files), not a tautology.
  Choices made there:
  - Every variable-table entry is one `Node` and keeps its table `index`; nested entries become children. A leaf value that
    holds nested untagged entries keeps them as `Part::Node` between `Part::Bytes`, so the table can be recomputed
    (`Document::table`) without understanding those values.
  - Fallback is local: a node whose shape does not match (wrong `SS` length, `BS` children that do not tile, name or
    type index outside NM, unknown tag) becomes `Kind::Raw` with a reason, its children are still built. A rare token
    never fails the save. `TokenStats` counts typed vs raw per tag and the reasons.
  - The `SS` length is not stored; it is derived from the body when writing, so a growing facts table needs no fix-up of it.
    A parse only accepts the typed form when `len == span size - 6`.
  - Values decode only when re-encoding gives the identical bytes (`Bool` 0/1, canonical packed strings); otherwise they
    stay opaque. Floats are kept as bit patterns.
  - Depth is limited to 256 (`MAX_DEPTH`; real saves reach 26) so a hostile table cannot overflow the stack.
  - `payload::tag_of` now reports only `BS VL OP SS` and the four-letter tags; other letter pairs are untagged
    sub-values (the old dump counted a packed string starting with `Q` as a `QB` tag).
  - Breaking change inside the workspace: `Region::bytes()` is replaced by `Region::to_bytes()` / `len()`.
  - Not decoded yet: array and struct values, `EngineTime`, `Vector`, `IdTag` and other fixed composites (opaque, round-trip),
    the untagged sub-values, and the `BLCK`/`AVAL`/`PORP` streams in non-facts `SS` bodies.
- First real-save run happened on 2026-11-04 (see `docs/research.md` Findings). It falsified three
  guesses: header ints are `[67, 29, 164]` (not 66/29/0), variable-table entries nest and are
  unordered, and the `MANU` body is not `u32 count + packed strings`. All three are now fixed in
  code: `payload::read_names` decodes `NM MANU u32 count, u32 0, count x (u8 len, Latin-1 bytes), u32 0,
  ENOD` and must consume the section exactly, which it does on all 73 distinct saves.
  Later that day 55 more 5.01 (`saveVersion` 67) saves from Nexus confirmed the container and variable-table findings on 56 saves.

## Facts (PHASE0 step 8, part 1, 2026-11-04)

- `facts::FactTable` is a pure value: `parse(body)` / `to_bytes()` are byte-exact on every 5.x save tested
  (56/56). Facts keep file order; `c1` and every other unknown field stay raw. Parse is strict (count must
  match, no leftover bytes, unique names, canonical packed-string names) so a table that parses always
  re-serializes identically; otherwise it returns an error and nothing is guessed.
- The blob is found by shape (`find_facts_blob`: the single `SS` variable whose body is `SXAP .. SBDF`), not
  by NM name or by index; none or several candidates is an error.
- `set_value` = "replace all records with one permanent record `{value, time, expiry -1}`" and appends a
  missing fact at the end; `add_record` appends and recomputes `c1` as "number of records with expiry >= 0".
  Both are HYPOTHESES backed by saves, not scripts or the game (see `docs/research.md`); `set_value` is what
  `Document::apply` uses (wired up since the rebase work, below).
- Values are accepted as `-32768..=65535` and stored as raw u16 bits (the i16 reading is a guess). Names must be Latin-1,
  non-empty and at most 128 chars; the writer never widens to UTF-16/UTF-8.
- `inspect` / `check-save` now parse the facts and run `check_ids` on the real fact names (`SaveInfo` gained
  `fact_count` and `fact_error`; old saves whose layout differs report `fact_error`, not a failure).
- `w3al facts <save> [--diff other] [--grep text]` replaces the planned `w3al dump --facts`.

## Facts, part 2: rebase and apply (PHASE0 step 8, 2026-11-04 late night)

Real vs hypothesis, so nobody mistakes the green tests for the gate (only step 9 is the gate):

- **Real, backed by 118 files:** `RB` = (NM name index, absolute offset) of each top-level `BS`; pointer block = offsets of the
  `NM` and `RB` tags + the two `SC` bytes (kept); footer = offset of the table. `Document::rebase` rewrites exactly those from the
  tree, checks the old `RB` names against the tree first, and returns `Unverified` for any layout it does not recognise (no
  guessing, the document is left as it was). `apply` runs `rebase` once BEFORE editing so a document it cannot rebase is
  refused untouched.
- **Real but a design choice:** `Document::edit_facts(|table| ..)` is the one way to change facts (parse the table, let the
  closure edit it, write it back, re-parse to check, rebase). `apply` and the research tool `w3al edit-fact` both use it.
  Failures inside the closure leave the document untouched.
- **Script check, 2026-11-04 night (no code change needed):** the game's `facts.ws` defines `FactsSet` as `FactsRemove` + `FactsAdd(ID, val,
  validFor or -1)`, so "one permanent record" is script-proven, not just inferred from diffs; values are signed in scripts
  (`FactsAdd(.., -1, ..)`, `ResetFactQuest`). Still native-only and therefore HYPOTHESES: the stamp `time`, `c1`, the i16 width on disk, and what
  `FactsDoesExist` says about a fact with zero records. `w3al edit-fact` gained `--remove NAME` (FactsRemove) for the gate-D saves
  (see `docs/research.md`, "script semantics of the facts API, and a visible-effect fact").
- **HYPOTHESES in `apply` (none tested in game; the first one is now backed by the script, see above):** `(name, value)` means FactsSet: one permanent record, `c1 = 0`; a new fact
  goes at the end of the table; `time` = newest record time in the table (`FactTable::latest_time`, not the true game clock);
  value is i16 on disk. A repeated name keeps its last value. Values outside `-32768..=65535` are refused.
- **HYPOTHESIS in the helpers:** `remove` is treated like "zero records" by the game (real saves hold zero-record facts);
  `set_records` recomputes `c1` by the inferred rule, `set_fact` copies it verbatim (used for exact restores).
- **Not done on purpose:** quest and journal edits (`Unverified`), `gameProp` > `gameplayFacts` (does it need to follow `facts`?),
  absolute offsets inside values (none found in a scan; the in-game load is the real check), the `.json` sidecar and container
  header (copied as is).
- **Export** now refuses placeholder effects (`placeholder = true` or any `PLACEHOLDER_` name) with `ExportError::Placeholder`
  before the template is read, so the sample questbook can never produce a save. `--allow-placeholders` (CLI only, not the app) is
  a development switch. `--stem` names the output like a normal save; stems are limited to letters, digits, `_`, `-`, space
  (no dots, no separators) and still go through `create_new`.
- `w3al edit-fact` (dev tool): `--set NAME=VALUE`, `--match-facts-of other.sav` (make every differing fact exactly equal to
  that save's, including removing facts), `--stem`, `--out-dir`. It re-reads the written file and verifies that only the
  requested facts changed, so a "VERIFIED" line means the file is internally consistent, NOT that the game accepts it.

## Journal editor (2026-11-04 late, gate J pending)

Design: `docs/findings/journal-editor-design.md`. Real vs hypothesis:

- **Real, proven offline:** strict reader of `CJournalManager > JActiveEntries` and the counters/tracking of `CWitcherJournalManager`
  (101 of 101 5.x saves); the diff and the edit (`journal::apply`) reproduce the game's own journal bytes (lukwraith 21 -> 22 both blocks
  byte-identical; `CJournalManager` byte-exact on 43 of 44 consecutive pairs); post-order table numbering (122 of 122 files), so `rebase` may
  now write a longer table after `renumber_post_order`.
- **Design choices:** entries are addressed by GUID path, never by index; new entries/counters are built from data (fixed layout) rather than
  cloned from donor nodes; `journal_append` is "ensure" (a template that already has the entry gets its status set); `from` guards a status
  flip; flags are derived, not stored in TOML; new counters go at a donor position (`at`) or at the end; `apply` works on a clone and
  verifies by re-reading. The questbook gets `journal_status` / `journal_append` / `journal_counters` / `journal_track` tables (lint
  validates them), filled only by `w3al journal-diff`.
- **HYPOTHESES (in-game gate J):** the game shows the quest log as stored; appended entries at the end and a counter outside hash order are
  accepted; a stale quest graph does not re-run a quest whose journal says Success; `Document::intern_name` (append a missing NM name,
  e.g. `JS_Failed`) is accepted. The last one is opt-in (`--allow-new-names`) and not used in gate J.
- **Still refused:** `Document::apply` / export with journal or quest effects (`Error::Unverified`); removing entries; quest-level
  `JS_Failed` as a product feature; `questSystem` and `community` edits; the unmodelled side tables (`JHuntingClues`, `JMonsterKnown`,
  `JEntryAdvancedInfo`).
- `w3al journal-set` (dev tool, like `edit-fact`) writes new files only and re-reads them; its "VERIFIED" means internally consistent, not
  accepted by the game.
- **Gate J result (2026-11-04, user-reported):** the quest log is shown as stored (J1/J2 show the quest completed), and the edited table
  loads; but a stale quest graph DOES re-run the quest (talking to Napp reopened it, with or without the quest's facts).

## Quest graph transplant (2026-11-05, gate G passed for mq0005)

Design: `docs/findings/quest-graph-design.md`. Code: `w3save::graph`, `w3al tree | graph-diff`, graph options of `journal-set`.

- **Real, proven offline:** a name-aware keyed tree diff of whole top-level blocks (children matched by block name + identity var:
  `fileName`, `tag`, `GUID`, `guid`, `p`, ...); `graph::transplant` (replace / remove / swap-remove / insert / recount by path, all-or-nothing,
  re-parsed and checked). Replaying the full `questSystem` + `community` diff of a consecutive pair on A gives B's blocks byte for byte on
  86 of 86 lukwraith pairs (the 2 others, 06 -> 07, are refused: the NM table is not a prefix). Count rules (`COUNT_RULES`: `numBlocks`,
  `numThreads`, `tagsCount`, `numCommunities`, `numStubs` = number of such children) and "every `ExternalDialog` guid is a waiting
  `questBlock`" hold on 101 of 101 5.x saves (112,536 checks). The mq0005 delta (four scopes + the scene list) transplants on 21 so that those
  nodes equal 22's and nothing else in the graph moves. `community > stubs` lose an entry by swap-remove (both real mq0005 completions).
- **Design choices:** paths, not indices, address nodes (`questThread[38424c39]`, `a#8` for unkeyed repeats); a transplant needs the
  template's NM names to be a prefix of the donor's and then appends ALL of the donor's extra names, so every index inside copied bytes
  (opaque values included) keeps its meaning; no name remapping is attempted. Counts are recounted, not copied, so a partial selection stays
  consistent. TOML deltas (`graph-diff --toml`) carry paths plus `before`/`after` subtree hashes as guards and the donor's stem; the subtree
  bytes are NOT in the TOML (applying needs the donor save). Graph deltas are a separate TOML table (`[effects.*.graph]`), not yet part of
  the questbook schema or lint.
- **Gate G (user-reported 2026-11-05, `C:\w3-out\gate_G`):** G1 (only the quest's graph scopes + its spawn set and stub, plus journal and six
  facts) loads, shows the quest done, Napp is gone, and talking to Willis does not reopen it; the re-saves keep the transplanted nodes. So, FOR
  THIS QUEST, the game runs a frontier assembled this way, removing a spawn set + stub despawns the NPC, and `universe` and the map pins could stay
  the template's. G2 (whole blocks) was not needed and not tested.
- **More gates passed (user-reported 2026-11-05):** P (Precious Cargo: the quest scope removed whole, the NPC kept and moved to a new phase),
  S (the spared-Napp branch from a donor the user played; the game then advanced the transplanted epilogue itself), F (granny / frying pan:
  a quest the template never started). R (two deltas from two different donors on one template) passed too.
- **Name remap (2026-11-05, offline):** `NameMode::Remap` translates NM indices of copied subtrees, decoding the opaque layouts that carry
  names and refusing the rest (`docs/findings/name-remap.md`); `journal-set --graph-remap`. Prefix mode stays the default and byte-exact.
  Gate M (2026-11-05) passed in game: a CName (`inputName`) remapped to an appended name index works and the game keeps our name table.
  HYPOTHESIS still: remapped struct field headers (`Vector`, `SActionPointId`) and `ActivatedInputs` (not exercised by M1).
- **Composition rule:** deltas meant to be combined use point ops on shared lists (scene list, spawn sets, stubs) and recount; a whole-list
  replace undoes other deltas. Guards are per delta; re-taking a guard on another template is allowed only for nodes the delta removes whole.
- **Still hypotheses:** other quests, longer play after a transplant, remapped copies in game.
- **Still refused / not built:** cross-playthrough donors (names differ: needs a typed name remap and a proof that opaque values carry no name
  indices); deltas from two different donors on one template (their appended names collide; same reason); export of graph effects.

## Export of quest effects (2026-11-05)

Design: `docs/findings/export-wiring.md` (written by the export subagent; verified here: 134 tests, clippy clean, the 6 fixture export tests
pass on real saves, and a manual `w3al export --run` of the R1 test questbook gives a payload identical to the in-game-tested gate R1).
- Questbook effects may carry `[effects.<key>.graph]` (`donor`, `source`, `ops`), the format of `graph-diff --toml`; one parser
  (`w3al-core::graph_effect`) for lint, export and `journal-set`. Graph paths are limited to `questSystem/` and `community/`.
- Donor saves are found by stem in folders from the run file (`donors = [...]`) or `--donors`; never in the repo.
- Per effect: graph, then journal, then facts; guards checked against the save as it is before that effect. Prefix mode when the donor's
  names extend the save's; remap only with `allow_name_remap` / `--allow-name-remap` (gate M passed for a remapped CName; other value types
  offline only). Overlapping op paths of two effects from different donors are refused.
- Still refused: placeholder effects, legacy `quests`/`journal` id lists, journal status names the template lacks (`allow_new_names` off),
  guard mismatches. `Document::apply` stays facts-only.
- Not done: no real questbook content uses graph/journal effects yet (Phase 3); no automatic re-guarding of a delta for another template.

## Format and export

- Chunk records are rebuilt on write with the absolute-end-offset hypothesis; the last record's end
  is 0 if the source file had a zero sentinel (default for new files, per the plan).
- `header_padding` between the table and `data_start` is preserved verbatim if its length matches.
- A damaged read (table overrunning `data_start`) is *reported* by `diagnostics`, not rejected, so
  `check-save` can describe it; export refuses such a template.
- Exporting a non-empty effect list: placeholder effects are refused (`ExportError::Placeholder`); real fact effects reach
  `Document::apply`; quest/journal effects still fail with `Unverified`. An export with no effects re-packs the template,
  which is the Phase 0 "does the game accept our container" check (user confirmed: it loads and plays).
- Default output name is a generated `ManualSave_<a>_<b>_<c>.sav` (**done**, `export::default_stem`, `save_name` module). In game the old `AnotherLife_YYYYMMDD_HHMMSS` name loaded but the Load list showed no quest name, while a `ManualSave_` name shows it (user-reported, research.md 2026-11-04). `a` is copied from the template's own name (it is a tracked-quest key; see `docs/findings/save-name-ids.md`), else the fixed `DEFAULT_QUEST_KEY`. `b`/`c` are the current date and time packed the way the game does, taken from UTC (**HYPOTHESIS**: the game uses local time; the Load-list ordering rule is untested), strictly later than the template's when its ids decode to a date, and never the stem of a `.sav`/`.png`/`.json`/`.req` already in the output folder. `--stem` overrides it (e.g. `AnotherLife_...`); `export::timestamped_stem` is kept only for that.
  Whether the payload's `saveInfo` `time` list must agree with the name is untested (the game loaded invented ids). Sidecars
  (`.png`, `.json`, `.req`) are copied from the template with the new stem.
- Until bundled act templates exist, the save the player picks on the Welcome screen is the
  template (`run.template`). Option B (six act templates) is still the plan.
- Spawn point: scaffold assumes the palace is a narrative "wake up" scene and the real spawn is a
  free-roam signpost (the plan's recommendation). Nothing writes a position yet.

## Questbook schema additions (beyond the plan's example)

- `default` is required on every choice (plan rule). `excludes` is supported and enforced.
- Added `started_effects` and `done_effects` (keys into the quest's `[effects.*]`) so "started" and
  the "done phase" have somewhere to live. Plan's `consequences` is not implemented; `requires` /
  `excludes` cover validation for now.
- Added `placeholder = true` on effects; lint reports each as a warning. Scaffold names use a
  `PLACEHOLDER_` prefix so none look like real identifiers.
- Lint rules: unique ids and unique `order`; `requires` must exist, be acyclic and have a smaller
  `order`; defaults must be options; every effect needs a `source`; unused effects warn.
- Soft level warning: a non-main quest more than **5** levels above the nearest main quest
  (previous, else next). The threshold is a guess (`validate::LEVEL_GAP`).
- Unknown TOML keys are rejected so typos fail.
- Sample quest levels (1, 1, 6), the White Orchard "reward" choice (a schema demo), and Family
  Matters' prerequisite (`beast_of_white_orchard`, standing in for the real chain) are **not verified**.

## Run semantics

- A quest missing from the run counts as skipped. `stop_after` = last quest included.
- Skipping or blocking a main quest locks later main quests; unmet `requires` (not completed/started)
  or a completed `excludes` blocks a quest with a warning.
- The Witcher 2 import is one boolean (`w2_import`); individual W2 decisions are an open question.
- Narration is hand-written TOML (plan's recommendation).

## App

- Tauri 2, `withGlobalTauri: true` (no bundler); native dialogs through `tauri-plugin-dialog`.
  CSP: `script-src 'self'`, so the UI never uses inline scripts or `style=` attributes.
- Backend is stateless: every command takes the run as input. `load_questbook` returns quests,
  level warnings and lint results so the UI re-implements no rules.
- "Stop here" is offered after every quest; `S` mid-quest stops *before* the current quest.
  "Started" is labelled experimental (open question).
- The quest image slot shows the region name when there is no image; nothing is bundled.
  How a player-provided image folder is served is not decided.
- Questbook is bundled as a Tauri resource (`questbook/`); dev checkouts find it by walking up from
  the working directory or executable (`W3AL_QUESTBOOK` overrides).
- Windows NSIS only; the Linux AppImage target and GitHub Actions are Phase 5.

## Repo

- Cargo workspace; `default-members` excludes `src-tauri` so `cargo build` / `cargo test` stay fast
  and need no WebView2. Build the shell with `cargo tauri build` or `cargo build -p w3al-app`.
- `rust-version = 1.82` (uses `Option::is_none_or`). `thiserror` 2, `toml` 0.8, `lz4_flex` 0.11.
- Licenses: code MIT OR Apache-2.0 (declared in Cargo metadata), questbook CC BY-SA 4.0. License files
  and the repository URL (`OWNER` placeholder in `Cargo.toml`) are not added yet.
- Not done from the plan: `insta` snapshots, `cargo fuzz` targets, `docs/playtests.md`, README install
  steps, CI. The parser returns errors instead of panicking and is unit-tested on truncated input.

## Act 1 scope (2026-11-05, decided by the user)

Witcher 2 import questions are out of v1 (the template's `import_*` facts stay); act 1 offers no choices, so no clean act-1 template or
side-quest donors are needed for v1. Details: `docs/plan/act1-white-orchard.md` section 7. The Phase 3 exit test drops "import choices show up".

## Act 2+ content model (2026-11-05, proposed, offline only)

`docs/plan/decision-chaining.md`: decisions are applied only where not yet made (no switching); acts are cut into **decision windows**;
each window has templates per earlier-window branch, parked before the window's decisions; stubs are addressed by `idTag`.
HYPOTHESES until gate X1: stub order is not semantic; a closure delta on an earlier sibling template runs in game; side quests can be
completed while the main chain is still active.

## Effects can remove facts (2026-11-05)
`facts_remove = [...]` on an effect, applied after `facts` (script `FactsRemove` semantics: the fact and all its records go; a name the
save lacks is skipped); verification treats the run's last request per fact as the wish. Needed because donors drop facts
(`keira_part1_dd_start` in every Keira branch).

## Synthesized effects (2026-11-05)
`[effects.<key>.synth] { start, through, pick, stop }` lets an effect be walked through the game's own quest graphs instead of
copied from a donor (`docs/plan/no-human-pipeline.md`, `docs/findings/export-wiring.md` section 8). Choices made:
- **Same semantics as `w3al synth-apply`.** Field names = the CLI flags (singular, lists; `scene_pick` = `--scene-pick`); listeners
  on, max 20,000 steps; all walk and write logic is `w3graph`'s. Both the CLI (`synth_cmd::build`) and `w3al-core::synth_effect::walk`
  turn their arguments into a `w3graph::synth::WalkSpec` and call `w3graph::synth::plan` (key resolution preferring the save's own
  frontier, socket check, starts with their saved input, And-block seeds from the frontier, listeners), so there is one walker glue.
- **One difference from the CLI:** a `start` block that does not wait in the save is refused (the CLI only prints a note). An export
  must not walk a decision from a save that is not at it; this is the synth counterpart of the graph guards.
- **Walk from the save as it is at the effect's turn** (template + earlier effects), not from the template: facts and frontier then
  include what earlier effects did.
- **Slot:** synth takes the `graph` slot (graph or synth, then journal, then `facts`, then `facts_remove`); `synth` and `graph` on the
  same effect are refused by lint and export. Explicit journal tables and facts stay allowed after the walk (scene-file facts and
  objective counters the graph cannot give).
- **Never a placeholder** (lint error): the keys come from the graphs.
- **Game folder:** `ExportOptions::game_dir` / run file `game_dir` / `w3al export --game-dir` / the app's Export page field, default the
  Steam folder. Read only when the run has a synth effect; loaded once per export (`export::load_graphs`; ~4 s from the bundles);
  `build_with_sources` takes preloaded graphs. Missing folder = `ExportError::Graphs` with the fix in the message.
- **Verification** adds: the walk's predicted frontier (new waits present, consumed/ended blocks gone, unless a later synth step changed
  them), its journal edit (counters included) and facts counted as requested, and exactly the top-level blocks `synth::apply` writes as
  touched: `questSystem`, journal, `facts`, `community` (spawn phases, sets, stubs) and `universe` (the `entityData` of removed stubs);
  everything else must stay byte-identical.
- **Not checked:** overlap between a synth effect and a donor `graph` effect on the same scope (paths are known only after the walk);
  the graph effect's guards catch most such cases.

## Story chain: automatic template, earlier decisions rewritten after the fact (2026-11-05)

- **Template choice is the app's, not the player's.** `questbook/templates.toml` registers the window templates on the
  pack line with what each already holds (`encodes`, from `w3al template-scan`) and what it cannot change (`fixed`). A run
  gets the latest template at or before its stop quest that fits; a hand-picked save stays as a manual override.
  Template saves are looked up by stem in folders (run `template_dirs`, default `templates` next to the questbook); they
  are never in the repo.
- **Rewrite = the difference between the two answers on the decision's own window template**, both exported by the
  normal pipeline, carried onto the later save: facts, journal entries (removed / appended / status), counters, and
  quest-graph / spawn-set state only while it is still live there; character-journal listeners are swapped, world-hub
  waits (persistent content, default dialogues, bug-fix phases, merchants) are never touched. HYPOTHESIS for the game:
  proven offline on three played branch pairs (0 differences), nothing loaded in game yet.
- **Journal entries may be removed.** `w3save::journal::remove` drops the node and lowers `Size`, keeping order: what
  the game itself did in the gate Y2 re-saves (5 entries, 1 counter). Export's own `verify_journal` still refuses removals
  inside a window's effects; only the rewrite step removes, and it verifies its own plan.
- **A decision a later quest already branched on is not rewritten** (`fixed`); the choice falls back to an earlier
  template and the app names the quests that are then not in the save. Judgement per decision in
  `docs/plan/story-chain.md` section 3; dialogue-only reads that already played (q310's Triss/Yennefer variant on pack 54)
  do not block a rewrite.

## Illustrations are linked, not stored (2026-11-05)

The user decided that quest and act illustrations may come from the Witcher Wiki on Fandom by **linking** to the
public image URLs (`static.wikia.nocookie.net`), shown in the app with a visible credit and a link to the wiki page.
No image file is stored in the repo or the installer; only the URLs are (CLAUDE.md rule 7 amended). Known trade-offs,
accepted: most wiki images are CD PROJEKT RED material the wiki itself hosts as fair use, so this is hotlinking, not a
licence; images need an internet connection and send requests to Fandom's CDN; links can break. The app falls back to
no image when a link fails, and offers a setting to turn online images off.
