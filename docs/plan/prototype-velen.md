# Prototype: Velen decision window W2 (Keira + Letho) through the questbook

Date 2026-11-05. Offline build and verification only; nothing here has been loaded in game yet. Labels as in
`decision-chaining.md`: **[proven]** = on real saves by a test or command below, **[hypothesis]** = needs the game.

## What it is

The first real questbook content: two act-2 quests whose choices map to captured graph + journal + fact effects,
exported from one template through the product path (questbook TOML -> `resolve` -> `export::export_with`, the same
code the app calls). It implements the "decision window" model of `decision-chaining.md` section D for W2.

* Template **T_keira** = `ManualSave_568ef_7ea51000_923cb5` (`C:\w3-fixtures\session_V1`; the user's 5.01 re-save of a
  Nexus 2394 pack save: q109 Active before Keira's final talk, sq102 not started; spirit destroyed and Baron hanged are
  fixed by the template, W1 is not offered).
* `questbook/02_velen/for_the_advancement_of_learning.toml` (order 2300): choice `keira` = `kaer_morhen` (default) /
  `radovid` / `killed`, effects `to_kaer_morhen` (25 graph ops), `to_radovid` (25), `killed` (19), donors `94abd5`,
  `97cdc9`, `96b66e`. The ops cover Keira's scope `a536c75d`, q109's own thread `bb109b9f` (pre-decision trigger
  `215f2719` out; `d5740aa4` and `f61db075` block+thread in, killed: `c8874cb7`), listeners `7ffa6436` / `973a4357`,
  scene `keira`, spawn set `keira_metz.w2comm` and her stub by idTag.
* `questbook/02_velen/ghosts_of_the_past.toml` (order 2310): choice `letho` = `kaer_morhen` (default) / `elsewhere`,
  effects `invited` (34 ops, donor `a4841f`) and `elsewhere` (35 ops, donor `9d341b`). Both are **closures** captured
  from T_keira (the donors were played from the later T_letho), so they also remove the start triggers `72a761d5`,
  `15601b24`, `8fdb44ab` that still wait on T_keira; `elsewhere` removes thread `6850c728` of hub `279a1a03` instead of
  inserting the wait `7573f7fb`. Donor names are not a prefix of the template's: `allow_name_remap` is required.

Capture (all from T_keira, commands in the quest file headers): `w3al graph-diff T D --stub-id --toml KEY --scope ...`
on the quest's own nodes (point ops on hubs, never a whole list); `w3al journal-diff T D` (Keira: whole diff; Letho:
`--grep letho` + `--grep sq102dolores`); facts from `w3al facts T --diff D` without noise (dialogue ids, `*_hit*`, timers,
tutorials, statistics, achievements, `mh305_merchant_spawned`, zero-record facts), multi-record facts summed. The
`invited` graph and journal tables are op-for-op the gate X deltas [proven: `diff` of the sorted ops]; the Keira KM q109
ops equal `scratchpad\synthw\keira_km_q109.toml`.

## The six saves

Folder `<scratch>/proto/<combo>/`,
each with `run.toml` (template, donor folders `session_V1` + `session_V2`, `allow_name_remap = true`, both quests
completed with the answers), `export.log`, and the save with its `.png` / `.json` sidecars. Built from the repo root with

```
cargo run -p w3al-cli -- export --run <combo>\run.toml --questbook questbook --out-dir <combo> --stem <stem>
```

(`<combo>` = the folder above; the stems give each file its own Load-list slot: invented times 03:01-03:06 on the template's date).

| combo | stem | Keira | Letho | what the game should show |
| --- | --- | --- | --- | --- |
| `km_invited` | `ManualSave_568ef_7ea51000_c100b5` | Kaer Morhen | invited | q109 past the talk, Keira bound for Kaer Morhen (q109 still Active, as in the donor); Ghosts of the Past completed, Letho gone from the farm, no restart at Dolores' farm or the notice boards; Letho expected at Kaer Morhen later |
| `km_elsewhere` | `..._c200b5` | Kaer Morhen | elsewhere | as above for Keira; Ghosts of the Past completed with Letho sent away (headhunters killed); no Letho at Kaer Morhen later |
| `radovid_invited` | `..._c300b5` | Radovid | invited | q109 past the talk with the Radovid objective / character entry (q109 Active); Letho as in `km_invited` |
| `radovid_elsewhere` | `..._c400b5` | Radovid | elsewhere | Radovid branch; Letho sent away |
| `killed_invited` | `..._c500b5` | killed | invited | q109 completed, Keira's spawn set and stub gone, tracking moved to the main quest; Letho invited |
| `killed_elsewhere` | `..._c600b5` | killed | elsewhere | Keira dead; Letho sent away |

Verification [proven offline, all six]: `w3al export` printed VERIFIED (re-parse, table, post-order, check-save, copied
subtrees equal donors, journal/facts as requested, nothing else changed, file re-read); `w3al check-save` no problems;
`w3al roundtrip` (km_invited) PASS. `km_invited` vs gate X1 (`C:\w3-out\gate_X`, rebuilt with the q109 ops):
`questSystem`, `community`, `CJournalManager`, `CWitcherJournalManager` byte-identical, NM table equal; facts equal
except `keira_part1_dd_start` (below).

Tests: `crates/w3al-core/tests/velen_w2_fixtures.rs`. `w2_quests_are_wired` runs without saves (lint, options ->
effects -> donors, no Keira/Letho op overlap). Fixture-gated: all six combinations exported and checked independently of
export's own verification (check-save, re-parse, names prefix, untouched top-level nodes, copied nodes = donor by
resolved fingerprint, removed nodes gone, journal entries/counters and facts as requested and as in the donor);
q109's thread equals each Keira donor's; KM + invited reproduces gate X1. Run with
`W3AL_FIXTURES=C:\w3-fixtures W3AL_DONORS="C:\w3-fixtures\session_V1;C:\w3-fixtures\session_V2" W3AL_GATES=C:\w3-out cargo test -p w3al-core --test velen_w2_fixtures`.

## Known risks

* **Fact removal is not expressible.** Every Keira donor removes `keira_part1_dd_start` and adds `keira_part1_dd_stop`;
  the questbook has no fact removal, so our saves keep `dd_start = 1` next to `dd_stop = 1` (gate X1 removed it with
  `edit-fact --remove`). Proposed src change (w3al-core, not done here): `facts_remove = ["..."]` on `Effect`, applied
  after `facts` through the existing `FactTable` removal, verified like set facts. [hypothesis: harmless, untested]
* **Donor timing.** The KM and Radovid donors were saved right after the decision, with q109 still Active; the game must
  finish q109 from the transplanted wait (`f5290e77` / `c133524a`). The killed donor was saved after the fight (q109 Success).
* **Letho closure while q109 is active.** In the pack Letho's quest came after the Keira decision; here both are applied
  to one template. Nothing in the graph refers to sq102 once its triggers are removed, but only the game shows that
  q109 and the Kaer Morhen waits (`f5290e77`, `7573f7fb`) behave. [hypothesis]
* **Remap.** Every Letho effect is remapped (1 name appended). Gate M proved remap for a CName; the closures also copy
  `Vector`, `SActionPointId`, `IdTag`, `ActivatedInputs`, which are proven offline only.
* **Side outcomes are baked in.** `invited` = headhunters let go, `elsewhere` = headhunters killed (the donors' play);
  the option texts say so. A free choice needs two more donors.
* **Left as the template's:** `universe` entity data of removed stubs (Letho; Keira when killed), as in gates G-R;
  journal counter positions (`at`) come from the capture saves (order only).
* **Only `km_invited` has a gate twin** (X1, in-game result pending). The other five combinations, Radovid, killed and
  elsewhere, have never been loaded. Suggested order: X1 first, then `killed_elsewhere` (the most different), then the rest.
* Levels in the quest files are unverified (`TODO(phase3)`), and "started" applies nothing for these quests.

In game, follow the gamesaves hygiene (back up, label, move the test saves out afterwards).
