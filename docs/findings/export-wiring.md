# Export wiring: questbook graph + journal + facts effects through `w3al-core` export

Date 2026-11-05. Builds on `quest-graph-design.md` (gates G, P, S, F, R passed in game), `journal-editor-design.md` and
`name-remap.md`. Labels: **[proven]** = in game (user-reported gates) or checked here on real saves by a test,
**[hypothesis]** = not shown in game.

Before this change only the dev tool `w3al journal-set` could apply journal + graph deltas; the product path
(questbook TOML -> `resolve` -> `export`) refused journal effects and had no graph table. Now the product path applies
them and reproduces the gate saves G1 and R1 exactly.

## 1. Schema: `[effects.<key>.graph]`

Exactly the table `w3al graph-diff --toml` prints and `journal-set --graph-from-toml` reads (`source`, `donor`,
`ops = [{op, path, before, after, of}]`). The parser now lives in `w3al-core::graph_effect` and is the ONE
implementation: questbook loading, lint, export and the CLI (`journal-set --graph-from-toml`, `--graph-op`) all use it.
`questbook/SCHEMA.md` documents it. Lint (`w3al lint`, via `GraphEffect::to_ops`) checks:

* op kind in `replace | remove | swap-remove | insert | recount`; unknown keys rejected (`deny_unknown_fields`);
* path: non-empty, no empty step, starts with `questSystem/` or `community/` (the two blocks whose transplant is proven
  in game; anything else, e.g. `universe` or map pins, is refused), not a whole top-level block;
* guards: `""` (unchecked) or `#` + 8 lowercase hex (the `graph::short_hash` shape); `before` only on replace / remove /
  swap-remove, `after` only on replace / insert, `of` only on recount;
* recount: path ends in `<parent>/<var>` of a `graph::COUNT_RULES` pair and `of` is that rule's child;
* `donor`: a save stem (letters, digits, `_`, `-`; no extension, no folders); required when an op copies
  (`replace`, `insert`); a delta of removals and recounts may omit it;
* `ops` non-empty; an effect with only a graph table is not "changes nothing";
* placeholder rules as before; a `PLACEHOLDER_` donor stem also counts as a placeholder at export.

Design choice: the questbook keeps one graph table per effect. Gate G's two files (quest scopes from donor 22, community
ops without donor) become one ops list in the test questbook; both are removal-or-copy ops of the same capture.

## 2. Donors

Donor saves are real saves, never committed and never in the questbook. Export finds `<donor stem>.sav` in donor
folders (not recursive):

* `donors = ["DIR", ...]` in the run file (`Run::donors`, serde default empty; `w3al export` resolves relative entries
  against the run file's folder), plus `w3al export --donors DIR` (repeatable), plus `ExportOptions::donor_dirs`;
  the app passes `ExportOptions::for_run(&run)`;
* a stem found in no folder, in two folders, or with no folder given is a clear `ExportError::Donor`;
* only effects whose ops copy need their donor file. mq0004's delta (donor lukwraith 23) only removes and recounts, so
  R1 exports without save 23 on disk; its `donor` stays as provenance (and for the overlap rule).

## 3. Apply order, name modes, composition

* Effects in run order (resolution order); per effect: **graph, then journal, then facts** (graph may append the donor's
  NM names, which journal edits then find). Same order `journal-set` uses.
* Guards are checked per effect against the save **as it is just before that effect** (the template plus earlier
  effects) and against the donor: a positional path (`community/stubs/a#8`) that an earlier swap-remove moved is caught,
  not mis-applied. A `replace` whose node already equals its `after` passes (same-donor repeat).
* Name mode per graph effect: no donor needed -> the save is its own donor (`NameMode::Prefix`, nothing copied, no names
  appended); donor whose names extend the save's (`graph::names_prefix`) -> `Prefix` (byte-exact, proven); otherwise
  `Remap` only with `allow_name_remap` (run file field or `--allow-name-remap`), else refused with a message naming the
  donor. **[hypothesis]** remapped copies in game (offline equality only; a gate is being prepared separately).
* Composition rule (DECISIONS.md): two effects whose op paths are equal or nested (one under the other) are refused
  unless both name the same donor. Recount ops are exempt (they count final children: how deltas share the scene
  list, spawn sets and stubs; R1's two deltas both recount `tagsCount`).

## 4. What export accepts and refuses now

* Accepts: `facts`, `journal_status` / `journal_append` / `journal_counters` / `journal_track`, `graph`.
* Still refuses: `placeholder = true` effects and `PLACEHOLDER_` names (`ExportError::Placeholder`, `allow_placeholders`
  is still dev-only); the legacy `quests = {...}` and `journal = [...]` id lists (`Save(Unverified)`); lint errors in
  journal or graph tables (`Invalid`); missing / ambiguous donors (`Donor`); guard mismatches, overlaps, non-prefix
  donors without remap (`Effect`); journal names the template lacks (`journal::apply`; `allow_new_names` stays off:
  appending `JS_Failed` etc. is unproven in game); damaged templates (`Template`).
* `Document::apply` is unchanged (facts only; still `Unverified` for quests/journal ids). Export calls it per effect for
  facts and calls `graph::transplant_with` / `journal::apply` directly; each of those is all-or-nothing and verifies
  itself.

## 5. Verification

`export::build_verified` checks the exact bytes that will be written, then `export_with` writes them (`create_new`) and
re-reads the file, which must be byte-identical to the verified bytes (else `Verify`, naming the file). Checks:
container decodes, typed re-parse serializes back to the payload, table recomputed from the tree equals the stored one,
post-order numbering (when the template had it or a graph/journal edit renumbered), `check-save` has no errors, the
template's NM names are a prefix of the result's; every top-level node no effect touches is byte-identical to the
template's (touched = the first path segment of graph ops, `CJournalManager`/`CWitcherJournalManager` for journal
effects, `facts` for fact effects; `MANU` grows by appended names); every replaced/inserted subtree equals its donor's
(`fingerprint`, or `resolved_fingerprint` in remap mode); keyed removed nodes are gone; every recounted count equals its
children; journal: no removals, every changed/added entry and counter was requested, each requested key ends at the
last requested status/unread/count, tracking as requested; facts: every changed fact was requested and holds the last
requested value.

## 6. Tests and results [proven offline]

`cargo test` (no env): 31 core unit tests (schema parsing, lint rules, overlap, donor lookup, refusals) plus the
existing ones; fixture tests skip. `crates/w3al-core/tests/export_fixtures.rs` with `W3AL_FIXTURES=C:\w3-extract`,
`W3AL_DONORS=C:\w3-fixtures\donor_S`, `W3AL_GATES=C:\w3-out`: 6 of 6 pass, driving `resolve` + `export::export_with` on
test questbooks in `crates/w3al-core/tests/data/gate_g1` and `gate_r1` (built from the gate delta files; not content):

* **G1** (template lukwraith 21, donor 22, one effect: 20 graph ops, 13 journal edits, 6 facts): the exported save's
  decoded payload is **byte-identical** to `C:\w3-out\gate_G\ManualSave_8559a_7ea50c00_50f4f2b.sav` (all 21,580 table
  entries, NM table 3,236 names, `questSystem`, `community`, both journal blocks, facts including record times, since
  both stamp the template's latest fact time).
* **R1** (template 21, two effects from two donors: mq0004 from lukwraith 23, removals only; spared Napp from donor S):
  decoded payload **byte-identical** to `C:\w3-out\gate_R\ManualSave_8559a_7ea50c00_54b3041.sav` (3,206 names, 21,387
  entries). With the two quests in the other run order, `questSystem` and `community` are still identical; the journal
  and facts hold the same content in another append order (expected: appends go to the end).
* Refusals on real saves: G1's delta on save 22 (guards), the brought and spared branches together (overlap), a second
  donor with other names (refused without remap; with `allow_name_remap` the copied subtree equals donor S's by
  resolved fingerprint).
* All 238 ops of the gate delta files (`C:\w3-out\gate_*\deltas\*.toml`) parse and pass the lint rules.
* `w3al journal-set` with the moved parser rebuilds G1 identically (0 regions differ); `w3al export` via the CLI gives
  the same G1/R1 saves.

## 7. Still hypotheses / open

* Remapped copies (`allow_name_remap`) in game; journal names appended by us (`allow_new_names`, not exposed in export).
* Guard semantics across templates: a delta is guarded to the template it was captured on (or re-guarded by hand, as
  mq0004 on 21); there is no automatic re-guarding.
* Same-donor overlaps pass the composition check, but each op must still find its node and pass its guard (a node an
  earlier same-donor effect removed is refused).
* The product still needs per-act templates and donors captured from them (quest-graph-design.md section 5); no real
  content uses graph effects yet; the sample questbook's `PLACEHOLDER_` effects are unchanged.
* `universe` entity blobs and map pins stay the template's (as in all gates).

## 8. Synthesized effects (`[effects.<key>.synth]`, 2026-11-05)

Donor-free effects: export walks a decision through the quest graphs of the player's own install, exactly what
`w3al synth-apply --start .. [--through] --pick .. --stop ..` does (`docs/plan/no-human-pipeline.md`), and writes the walk
into the save. Schema in `questbook/SCHEMA.md`; choices in DECISIONS.md "Synthesized effects".

* **Code.** `w3al-core::synth_effect` (table, lint shape, adapter) calls only w3graph's public API: `frontier::read`,
  `frontier::fact_sums`, `walk::{Policy, Walker}` (`run`, `fire_listeners`), `frontier::predict`, `synth::journal_edit`,
  `synth::apply`. ADAPTER: the CLI's policy builder (`synth_cmd::build`: key resolution preferring the save's frontier for
  short keys, socket check, starts with their saved input) is private to the CLI, so `synth_effect::walk` repeats it; a
  start not waiting in the save is refused (the CLI only notes it).
* **Order.** Per effect: `graph` or `synth`, then journal tables, then `facts`, then `facts_remove`. The walk starts from
  the save as earlier effects left it.
* **Graphs.** `ExportOptions::game_dir` (run file `game_dir`, `w3al export --game-dir`, app Export page "Game folder";
  default `export::DEFAULT_GAME_DIR`, the Steam folder). `export::load_graphs` loads them once per export, only when the run
  has a synth effect (~4 s from the bundles); `build_with_sources` takes them preloaded. Missing: `ExportError::Graphs`.
* **Verification** (on the built bytes, besides section 5): every new wait the walk predicts is in the frontier and every
  block it consumes or ends is gone (`frontier::predict`; unless a later synth step changes that block), the walk's journal
  edit counts as requested (each entry at its status, nothing else changed), its facts hold the values `synth::apply` set
  and its removed facts are gone (then the explicit facts on top), and only `questSystem`, the journal blocks and `facts`
  may differ from the template among the top-level nodes.
* **Results [proven offline].** `crates/w3al-core/tests/synth_effect_fixtures.rs` (`W3AL_FIXTURES=C:\w3-fixtures`, install
  at the default path or `W3AL_INSTALL`; 3 tests, ~19 s): a test questbook with Keira to Kaer Morhen and Keira to Radovid
  as synth effects (`start 215f2719, through, pick d24f649d=Keira to KM | Keira to Radovid, stop 4c2a9b2d`) exported from
  T_keira through `resolve` + `export_with` with no donor folder: the journal has **0 differences** to the played donors
  94abd5 / 97cdc9; every fact the export changed equals the donor's (`q109_keira_to_km` / `q109_keira_to_radovid`,
  `q109_confronted_keira`, `keira_part1_dd_stop`, `keira_part1_dd_start` removed); verify passes. Also: explicit
  `facts_remove` applies after the walk; one graph load serves two builds; a save already past the decision is refused
  ("not waiting in the save"). `w3al export` with such a questbook prints the synth summary and the `VERIFIED` line.
* **Not in game yet** (gate K3): the synthesized frontier, and the missing community / counter / scene-fact parts listed in
  `no-human-pipeline.md` section 2.
