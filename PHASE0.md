# Phase 0: save-format spike (go / no-go gate)

**Goal:** prove we can read a real 5.01 save, re-serialize it, and change one fact that the
game then shows. If this fails, stop and rethink the project.

**Be honest about what the scaffold can do today.** Steps 1-5 below work now and check the
container layer plus our assumptions about the payload layout. They do **not** yet prove the
gate. The typed tokenizer (step 7, done) now makes the parse -> serialize round trip a real test of
`crates/w3save/src/token.rs`, but only step 9 (an edited save that the game loads and shows) is the
gate. Do not mark Phase 0 passed before step 9.

## 0. Prepare (never touch originals)

1. Game: Witcher 3 Remastered, patch **5.01** (shown on the main menu). Note DLC state.
2. Saves live in `Documents\The Witcher 3\gamesaves` (verify on your machine).
3. Make a fixtures folder and copy saves **with their sidecars** (`.sav`, `.png`, `.json`):

   ```powershell
   $env:W3AL_FIXTURES = "C:\w3-fixtures"
   New-Item -ItemType Directory -Force $env:W3AL_FIXTURES
   Copy-Item "$env:USERPROFILE\Documents\The Witcher 3\gamesaves\*" $env:W3AL_FIXTURES
   ```
4. You need **5 or more saves from different points** (for example: a fresh start, after the
   White Orchard griffin, mid-Velen, Novigrad, late game). Manual saves are best.
5. `cargo build -p w3al-cli` (binary: `target\debug\w3al.exe`).

## 1. Check for damage (5.0.0c bug)

```powershell
cargo run -q -p w3al-cli -- check-save "$env:W3AL_FIXTURES\<save>.sav"
```

**Pass:** no `ERROR` lines. A save made on 5.0.0c can be 96 MB with 266 chunks; do not use it
as a fixture (also do not use it as a template).

## 2. Dump the structure

```powershell
cargo run -q -p w3al-cli -- dump "$env:W3AL_FIXTURES\<save>.sav"
cargo run -q -p w3al-cli -- dump "$env:W3AL_FIXTURES\<save>.sav" --json > dump.json
```

Record in `docs/research.md`, per save:

| Expect (plan) | Where in the output |
| --- | --- |
| 5-6 chunks, data start 3084, payload about 5 MB | first lines |
| header ints `[67, 29, 164]` on 5.01 (saveVersion, gameVersion, ?); `looks like 5.x: true` | `header ints` |
| footer parses; variable count about 75,000 on 5.01 (observed 75,566) | `variables` |
| `NM @` and `RB @` offsets plausible; note `pre-table bytes` | `pointers` |
| names (NM) reads without error | `names (NM)` |
| tags like `BS SS VL MANU` dominate | tag histogram |

Every `[warn]`/`[note]` line is a falsified hypothesis. In particular **`names (NM) not read`
means the NM/MANU layout guess is wrong**: open Atvaark's `VariableParsers/` (MIT) and fix
`payload::read_names`. Paste the exact message into `docs/research.md`.

## 3. Round trip

```powershell
cargo run -q -p w3al-cli -- roundtrip "$env:W3AL_FIXTURES\<save>.sav"
```

**Pass:** both lines say `PASS`. "compressed bytes equal to original" will almost certainly be
`false`; that is expected (different LZ4 encoder) and is not a goal.

Repeat on every fixture. Any `FAIL` or error is a finding: write it down before fixing.

## 4. In-game check A: does the game accept our container?

This isolates the container writer (chunking, table, padding) from everything else. An
export with **no effects** re-packs the template unchanged.

1. Create `empty.run.toml`:

   ```toml
   version = 1
   template = "C:\\w3-fixtures\\<save>.sav"
   ```
2. Export into a scratch folder (not the game folder yet):

   ```powershell
   cargo run -q -p w3al-cli -- export --run empty.run.toml --out-dir C:\w3-out
   ```
   It writes `ManualSave_<a>_<b>_<c>.sav` (a from the template, b/c the current date and time) plus copies of the template's `.png`/`.json`.
   It never overwrites anything.
3. `cargo run -q -p w3al-cli -- roundtrip C:\w3-out\ManualSave_*.sav` should still PASS.
4. Copy the new set into `Documents\The Witcher 3\gamesaves`, start the game, open **Load**.

**Pass:** the save is listed, loads, you can move, and an in-game save made afterwards
also loads. **Record** what the list shows (name, date, thumbnail): this tells us which of the
three hex ids in the file name the game keys on (research: sidecars and naming).
**Fail:** note the symptom (not listed / refuses to load / crash). Compare header, table and
padding against the original with `w3al dump --json`.

## 5. Find a fact by diffing

1. Pick one cheap, observable decision (one dialogue option with a visible later effect).
2. Make a manual save right **before** it and one right **after** each option.
3. `cargo run -q -p w3al-cli -- diff before.sav after.sav` lists the regions that changed.
   Facts-level diff arrives with the facts parser; for now note the changed variable
   indices and sizes in `docs/research.md`.

## 6. Development: make the NM/MANU reader match reality (dev task) - DONE 2026-11-04

`payload::read_names` now decodes the name table on all fixtures and the section ends exactly
at the pointer block (table offset - 10, not the RB offset: RB sits before NM). Unit test uses
39 real bytes; `tests/names_fixtures.rs` runs on `W3AL_FIXTURES`. See `docs/research.md`.

## 7. Development: typed tokenizer (dev task) - DONE 2026-11-04

Port one parser per token type (Atvaark's `VariableParsers/`, MIT, with attribution) into
`w3save`, replacing `Region::Variable` bytes with nodes. Exit: parse -> serialize reproduces
the decoded payload on **5+ saves**, and `cargo test` has a fixture-gated test for it.

Done: `crates/w3save/src/token.rs` (typed `BS`/`VL`/`OP`/`SS`/`MANU` nodes, facts `SS`, raw fallback for
anything unrecognised). `tests/tokens_fixtures.rs` asserts typed parse -> serialize == decoded payload
**and** that the variable table recomputed from the tree equals the on-disk one; it passes on 101 of 101
files (73 distinct saves). Run it with `W3AL_FIXTURES` set; `w3al dump` prints typed-vs-raw coverage per
tag (94.85% of entries typed, every tagged token typed, the rest are untagged sub-values). This is
still not the gate: nothing has been edited or loaded in the game (step 9).

## 8. Development: facts locator, one edit, rebase (dev task)

1. Implement `facts::parse` (find the section, the per-fact record-count rule) and a
   `w3al dump --facts` listing; confirm ids are short and Latin-1. **DONE 2026-11-04**:
   `facts::{find_facts_blob, FactTable::parse/to_bytes}` plus `w3al facts <save> [--diff b.sav] [--grep text]`.
   The `SS` body is byte-exact on 56 of 56 Nexus saves (5.0 / 5.01) and `SBDF` count == facts parsed;
   `tests/facts_fixtures.rs` runs on `W3AL_FIXTURES`. Fact `set_value` / `add_record` are pure
   functions (wired into `Document::apply` in part 2). See `docs/research.md`, "facts blob".
2. Implement `Document::apply` for facts and `Document::rebase` (variable table, RB offsets, pointer block, footer).
   **DONE 2026-11-04 (late night)**, unproven in game. `RB` entries are (NM name index, absolute offset) of the top-level
   `BS` nodes (118 of 118 files); `rebase` rewrites table, `RB`, pointer block and footer from the tree and is a byte-exact
   no-op on unedited documents (`tests/rebase_fixtures.rs`, `W3AL_FIXTURES`; growth, shrink and no-op on 15 real saves).
   `apply` = FactsSet semantics through `Document::edit_facts` (see `DECISIONS.md`, "Facts, part 2", for every remaining
   HYPOTHESIS). Quest and journal edits stay `Unverified`. Dev tools: `w3al edit-fact --template a.sav --set NAME=VALUE
   [--match-facts-of b.sav] [--stem NAME] --out-dir DIR` (writes a NEW save + sidecars, re-reads and verifies it) and
   `w3al export --stem NAME`. Placeholder effects never reach an export.
3. Confirm with the diff from step 5 that only the intended fact changed. **DONE for the tool output**: `w3al facts a --diff b`
   on the gate saves shows exactly the requested change; the confirmation that matters is on the save the GAME writes (step 9).

## 9. In-game check B: the gate

1. From a save taken just **after** the observable decision in step 5, produce a copy where the
   fact is set back to its **before** value.
2. Load it in game.

**Pass (Phase 0 done):**
- round trip identical on 5+ saves from different points (step 7), **and**
- the edited save is listed, loads without crashing, and the world shows the change
  (for example the decision can be made again, or the dialogue/journal state matches the
  "before" save), **and**
- a new in-game save made from the edited one loads, and `check-save` on it is clean.

**Fail:** record why in `docs/research.md` and stop. Do not start Phase 1 on a failed gate.

### What step 9 needs from the user now (prepared 2026-11-04, nothing here is passed yet)

Three ready-made test sets are in `C:\w3-out\gate` with a `README.txt` (copy them into
`Documents\The Witcher 3\gamesaves`, nothing is overwritten):

| Set | What it is | What it proves if the game is happy |
| --- | --- | --- |
| `gate_A_newfact` | your own save + one new fact (`w3al_gate_a_test_fact` = 1); the facts block grew 36 bytes, so every offset was rebased | the game accepts rewritten offsets and a new fact |
| `gate_B_revert` | lukwraith 06 with the six facts that differ from 05 restored to 05's exact state | exact restore, removal and re-adding of facts on a second save |
| `gate_C_stem` | your own save, unedited, named `ManualSave_52586_7ea4cc00_10309b7` | whether the `ManualSave_...` name decides the quest name in the Load list |

Per set: does it show in the Load list (and with what label), load, play, and does an in-game manual save made from A / B
load. Then the objective check on that new in-game save: `w3al facts <new save> --grep w3al_gate_a` must still list the fact
(A), and `--grep q001_gwint_game_won` must still show ONE record (B). This is the "world shows the change" evidence for B;
its visible effect in game may be nil (tutorial / achievement facts), so the re-read is what counts. Phase 0 is passed only
when these results are in and recorded in `docs/research.md`.

### Visible-effect check (prepared 2026-11-04 night): `C:\w3-out\gate_D`

A and B proved that the game reads our facts table (its own re-saves still hold our edits). What was still missing is a visible
effect of an edited fact. D1 / D2 change `gwent_difficulty`, which the pause menu (Options > Gameplay) shows as Easy / Normal / Hard
straight from `FactsQueryLatestValue`; D1 sets it with `apply` (Hard), D2 removes it (the game's own `OnSpawned` re-adds it as Normal).
Evidence and script lines: `docs/research.md`, "script semantics of the facts API". README with the exact steps is in that folder.
Phase 0 stays NOT passed until the in-game results are recorded.
