# Research log and open questions

Every claim about the save format is a hypothesis until a test on a real 5.01 save proves it.
Record findings under the item they settle, with the save's patch, DLC state and how you checked.

## Phase 0 checklist (from the plan)

### Container (`SNFHFZLC`)
- [x] Magic, u32 count at 8, u32 data start at 12 confirmed (17 saves, one on 5.01).
- [x] Data start is 3,084 on every fixture; chunk count is 5-6 on healthy saves.
  - Triage 2026-11-05 (ticked = settled, statement partly refuted): data start 3,084 holds; the chunk count does not (1-18 chunks, see "2026-11-04 (later)": 13k-215k variables, early saves have one chunk). Settled, correction only.
- [x] Chunk record end offset is absolute (data_start + cumulative); confirmed on 17 saves.
- [x] Last record's end offset is 0 (sentinel) on every fixture.
  - Triage 2026-11-05 (ticked = settled, statement refuted): refuted. 5 of 56 saves and the game's own single-chunk saves carry a stale end offset (see "2026-11-04 (evening): first in-game results"); we write 0 or recompute and the game loads both (gates A, B, C). Don't-care.
- [x] Bytes between the table and `data_start` are zeros.
  - Triage 2026-11-05 (ticked = settled, statement refuted): refuted on 5 of 56 saves; padding kept verbatim when its length matches; loads in game (same entries as above). Don't-care.
- [x] Our re-compressed file loads in game (unedited repack). User-reported 2026-11-04: `AnotherLife_20261104_172834.sav` (repack of one of their own 5.01 saves, sidecars copied with the new stem) loads. Play: fine. A new in-game save made from it also loads: fine (user-reported). Load list: the entry appears but WITHOUT the quest name that normal saves show (the user's reading: it looks odd). Cause unknown. Candidates, none tested: (a) the label is derived from the `ManualSave_<a>_<b>_<c>` file-name ids, (b) it comes from the save payload or the journal tracking data, (c) it needs a field in the sidecar that our copy lacks. Note the lukwraith `.json` sidecars are all byte-identical and hold no quest, so the .json is unlikely to carry it. Cheap test: export the same repack under a `ManualSave_...` stem and compare the list entry.

### Payload (`SAV3`)
- [x] Header ints on 5.01 are [67, 29, 164] = saveVersion, gameVersion, (unknown; not buildPatch). Confirmed by the .json sidecar. See Findings.
- [ ] Where `magic_number` and `runtimeGUIDCounter` live in the header; what the counter must be in an exported save (new entities take IDs from it).
  - Triage 2026-11-05: `magic_number` / `runtimeGUIDCounter` are NM names but no doc locates them or tests a counter value. Exports that add entities (start kit items, synthesized stubs; gates Y, Z) loaded and re-saved fine, so it has not bitten. Remaining: locate them, decide whether a counter bump is ever needed.
- [x] Footer = u32 variable-table offset + `SE`; offsets are absolute (include the 3,084-byte header).
- [x] The two u32 at table offset - 10 are the absolute offsets of the `NM` tag and the `RB` tag (proven on 118 of 118 files, rebased correctly by `Document::rebase`); the two bytes after them are `SC` on every save, meaning unknown, kept verbatim.
- [x] Variable table = u32 count + (u32 offset, u32 size) pairs. NOT sorted by offset and entries nest (parent spans contain children); top-level spans tile the payload. The table recomputed from the typed tree equals the on-disk table on 101 of 101 files (2026-11-04, typed tokenizer). Entry count is 13k-215k, not 76-81k.
- [x] **NM/MANU encoding** (2026-11-04, see Findings): `NM` `MANU` u32 count, u32 0, `count` x (u8 len, Latin-1 bytes), u32 0, `ENOD`; ends exactly at table offset - 10. Decodes on 73 of 73 distinct saves. `payload::read_names`.
- [x] Name table use (2026-11-04, typed tokenizer): `BS` carries a 1-based u16 name index; `VL`/`OP` carry 1-based name and type indices. Every `BS`/`VL`/`OP` on 101 files has them in range. No NM name is a fact id (fact ids are packed strings inside the facts `SS`); the top-level `BS` named `facts` holds that `SS`.
- [x] `RB` = `"RB"`, u32 count, count x (u16 NM name index, u32 absolute offset): one entry per top-level `BS` in file order (118 of 118 files, 2026-11-04 "RB decoded"). Not a size.
- [x] Token layouts (2026-11-04, see Findings "typed tokenizer"): `BS`, `SS`, `VL`, `OP`, `MANU`, `SXAP`, `SBDF`/`EBDF` byte-exact on 101 files. `AVAL`, `PORP`, `BLCK` are not table entries: they sit inside non-facts `SS` bodies and are only partly checked (kept as blob bytes).
- [x] Typed parse -> serialize reproduces the decoded payload byte for byte on 101 of 101 files (73 distinct saves, 5.01 and older), and the table recomputed from the tree equals the on-disk one (`tests/tokens_fixtures.rs`). PHASE0 step 7 done; the in-game gate (step 9) is NOT passed.

### Facts and quests
- [x] How the facts region is addressed (2026-11-04, see Findings "Facts blob"): not by NM name but by shape. It is the one variable-table entry tagged `SS` whose body starts `SXAP` + 3 ints + `SBDF`, nested in a `BS` node. `facts::find_facts_blob`. 56 of 56 saves from 5.0 / 5.01.
- [x] Per-fact record count rule: an explicit u16 `n` after the identifier and a u16 `c1`; `n` x 10 bytes follow. `n` can be 0 (1,211 of 1,992 facts on the last lukwraith save) and as large as 2,463. `SBDF` count == facts parsed, nothing left over, on 56 of 56.
- [x] Record fields (10 bytes): u16 value, f32 game time, f32 expiry (`-1` permanent, `-2` dialogue ids, else a real expiry time). The value is **not** the first i32; it is a u16 (65535 occurs, probably i16 -1: unproven). `c1` = number of records with `expiry >= 0` on 57,955 of 57,955 facts: inferred from data, not from a script.
- [x] Fact ids are Latin-1 packed strings (ANSI bit set and canonical on every fact; no non-ASCII name in the lukwraith set); `check_ids` is quiet on all 56 saves, longest name 71 chars. `check-save` now runs it on the real fact names.
- [x] Is quest state fully expressed by `questSystem` + facts, or must `CJournalManager` also be patched for a quest to show as complete? (Plan unknown 3.) Offline answer: the journal is separate stored state (`docs/findings/quest-state.md`); a journal editor exists and reproduces the game's own journal bytes (`docs/findings/journal-editor-design.md`). In-game gate J (`C:\w3-out\gate_J`) pending.
  - Triage 2026-11-05: no, facts + journal are not enough; quest state = `questSystem` graph + `community` + journal + facts. Gate J/J2 (2026-11-04 night), gates G, P, S, F, R, M passed 2026-11-05 ("2026-11-05: the quest graph", "gate G result").
- [x] One fact edited with `w3al`, save loaded, change observed in game. (Gate saves A and B written 2026-11-04 in `C:w3-outgate`; in-game result pending, see PHASE0.md step 9.)
  - Triage 2026-11-05: done. Gate D (2026-11-04 night, "gate D result and Phase 0 verdict"): `gwent_difficulty` 3 = Hard, removed = Normal; gates A and B re-saves keep the facts.

### Sidecars and naming
- [ ] Exact 5.01 sidecar set (`.png` + `.json`?). Contents of the `.json`; does the game validate it against the `.sav`?
  - Triage 2026-11-05: set is `.sav` + `.png` + `.json` on 5.x (2026-11-04 first run); copies with a new stem load in all gates. Remaining: does the game validate the `.json` against the `.sav` (never varied), does a missing `.json` or `.png` matter (not tested).
- [ ] Which of the three hex ids in `ManualSave_<a>_<b>_<c>` the game keys on, and what makes a new file show as a distinct entry in the Load list. (We currently copy sidecars with a new stem.)
  - Triage 2026-11-05: answered in `docs/findings/save-name-ids.md` (`b`/`c` = packed local date/time, `a` = tracked-quest key; invented ids and the `ManualSave_` shape are accepted, the shape is what shows the quest name). Remaining: Load-list ordering rule (name stamp vs payload time vs file time) and our UTC stamp vs the game's local time (DECISIONS.md "Format and export": hypothesis).

### Position and world
- [x] How player position and the loaded world/level name are stored (`universe` / entity data). (Plan unknown 4.)
  - Triage 2026-11-05: `docs/findings/position-world.md`; in game T1 (same-world position), T2a/T2b (world swap into a visited world) loaded ("2026-11-04 (late): position and world-swap tests"); player position used again in gates Z. Open sub-points are in the triage table (long play after a swap, never-visited worlds, spawn choice).
- [ ] Does a template made under 5.01 still load after a future patch bumps saveVersion? (Plan unknown 5.)
  - Triage 2026-11-05: still open and not testable until a patch after 5.01 exists. The 5.0 -> 5.01 step (`saveVersion` 66 -> 67) parsed identically offline.

## Open design questions (plan, "Open questions for reviewers")

- [ ] **Spawn point:** literal Vizima palace spawn (risk: stuck Geralt) or palace as narrative scene with a free-roam signpost spawn? Scaffold assumes the latter (DECISIONS.md).
  - Triage 2026-11-05: format side answered (position edit and world swap proven, `position-world.md`; the palace is the separate world `wyzima_castle`, a literal spawn needs a world block). Design decision still not taken, and X1 showed the player spawning in the middle of the river near the tower (template position). See triage table.
- [x] **Template count:** one template (A) or act templates (B)? Who captures them, on which patch? Scaffold assumes B but currently uses the player's own save as the template.
  - Triage 2026-11-05: decided by the user and built: window templates per act ("decision windows", `docs/plan/decision-chaining.md`), registry `questbook/templates.toml`, templates synthesized from pack saves where none exists (`docs/plan/story-chain.md`); gate Z passed in game.
- [ ] **Patch support:** pin to 5.01 or also 4.04?
  - Triage 2026-11-05: v1 is pinned to 5.01 (`docs/PLAYER_GUIDE.md`: tested on 5.01 only). 4.04 and other patches are neither decided nor tested; old-gen `[64,18,163]` saves share the container and mostly the typed tree but 11 of 16 fail the facts layout offline (2026-11-04 night).
- [ ] **Quest coverage:** proposed rule - any quest that changes an ending factor, a Kaer Morhen ally, or a character's survival.
  - Triage 2026-11-05: in practice coverage = the decision catalogue (`docs/plan/decision-catalogue.md`, 17-20 questbook quests) plus 98 generated side quests; no doc states the final rule. Remaining list is in the triage table.
- [ ] **"Started" status:** is a quest left at its first phase stable in game? The UI labels it experimental.
  - Triage 2026-11-05: mid-quest frontier states are stable in game (X1 Keira's quest continues, K2 `c133524a` still waiting, S1/R1 epilogues advanced by the game); a never-started quest left at its first phase was not gated on its own (F completed it). UI label may stay "experimental".
- [x] **Witcher 2 import:** the game's own import questions (stored as facts) or every W2 decision? Scaffold has one on/off switch (`w2_import`).
  - Triage 2026-11-05: decided by the user, out of v1 (DECISIONS.md "Act 1 scope"); the template's `import_*` facts stay.
- [x] **Narration:** hand-written (recommended for v1) or LLM at runtime?
  - Triage 2026-11-05: hand-written TOML (DECISIONS.md "Run semantics").
- [ ] **Name and license** confirmation (MIT/Apache-2.0 code, CC BY-SA questbook).
  - Triage 2026-11-05: code MIT OR Apache-2.0 and questbook CC BY-SA 4.0 are in the Cargo metadata and `questbook/LICENSE`; DECISIONS.md "Repo": top-level license files and the repository URL (`OWNER` placeholder) not added yet.

## Open items (triage 2026-11-05)

Ranked by how much a player would notice. "No human" = can be done without anyone playing the game. Items marked (brief) come from the
triage request, not from a repo doc. Effort is rough.

| # | Item | Why it matters | Kind | Effort | Blocker | No human? |
| ---: | --- | --- | --- | --- | --- | --- |
| 1 | Side-quest completion (`complete = ...`, `run.side_quests`, 98 generated quests) was never loaded in game: exports are VERIFIED offline only (30 on pack 43, 30 on pack 54). Includes the dropped `suspendedScopesData` entry (far-world templates) and a contract finished while its world is loaded | The whole Side quests page rests on it; a bad suspended-entry drop could crash or reopen quests | in-game test | 1-2 h of play, 3-4 saves | none | No (a runner mod would make it Yes, item 14) |
| 2 | Frozen-quest trick (a quest held at a phase while the rest advances) never loaded in game; "started" for a quest never begun was not gated on its own (F completed one) | Player-visible "in progress" states and the UI's "experimental" label | in-game test | 30 min | none | No |
| 3 | Player start position / spawn: X1 spawned Geralt in the middle of the Velen river (template position). Position edit and visited-world swap are proven (T1, T2a, T2b), but no rule picks a safe spot per act, the palace-vs-signpost decision is open, and long play after a swap, a never-visited world, and horse / boat state at save time are untested | First thing a player sees; a drowned or stuck start is a bad first run | engineering + in-game test | 1-2 d (spot catalogue per world from real saves, then 1 gate) | spawn design decision | Partly (catalogue yes, gate no) |
| 4 | Decisions not settable that need only player re-saves: Keira's date (pack 12), Carnal Sins (pack 29), A Towerful of Mice (pack 11), Cerys donor (pack 36), Vigi saved (pack 34), Lambert (pack 47), Sara / haunted house (pack 17). Registry entries exist for Carnal Sins and Towerful, no 5.01 template yet | Each is a named choice players expect; `sq101_keira_had_sex` sits on every later template | human re-save (load in 5.01, save at once), then content | 30 min of the user's time, then 1 d content | the user | No for the re-saves, then Yes |
| 5 | Decisions the walker cannot do yet: Cerys queen (walk now reaches `q206_cerys_queen`, 1,626 steps, not exported or compared with a donor), Ves (An Eye for an Eye: fate is combat, kill phase leaves no waits), botchling (needs a save inside Family Matters before the ritual), Vigi saved (a race; the through-walk takes one branch), Lambert (needs a save inside the battle), Sara / Greedy God / Woods / Wolf's Clothing / Dangerous Game (not reached) | Missing story choices; Cerys and Vigi change later acts | offline research + engineering | 2-4 d | mid-quest saves for botchling and Lambert | Cerys yes; the rest need saves |
| 6 | The 171 side quests the auto-complete generator cannot do yet: 90 no quest graph (journals written by scripts, scavenger notes, points of interest, boat races), 36 not running on any template, 17 no route to Success, 11 decision quests, 5 unresolved, 1 phase still running, 11 done on every template (not a gap) | Most of the base-game side list is absent from the Side quests page | engineering: journal-only mode for graphless quests, hub-gate walk for not-running ones, fold decision quests into the decision questbook | 3-6 d | none | Yes |
| 7 | Kaer Morhen allies (4 invitations, template synthesized from pack 43) and other story-chain rewrites: exports VERIFIED, never loaded (only Z1 / Z2 played); Y4 / Y5 accepted without play | Brothers in Arms and the council scene are core act-2/3 beats | in-game test | 1 h | none | No |
| 8 | Hearts of Stone / Blood and Wine: out of v1 (CLAUDE.md); 109 expansion quests skipped by the generator; DLC state in `AdditionalContent` and mod checks in export (STATE next task 3) | Large part of the story; many players own both | content + research | weeks | scope decision | Mostly |
| 9 | Autogen / looted gear in the start kit: refused today (ability list order and jitter, NG+ `autogen_fixed_*`); kit uses fixed-stat pieces only; `z = 1` enchant records not decoded; uid allocation rests on statistics (`docs/findings/character-inventory.md`). Gate Z loaded the fixed kit | "Recommended gear" tier may not match what players expect | offline research | 1-2 d | none | Yes (compare with the game's own saves) |
| 10 | Template ordering limits: questbook order vs pack line order (for example Now or Never before Skellige act 2, while the line is pack 33-35 / 42); `fixed` decisions a later quest already branched on fall back to an earlier template and the app names the lost quests; King's Gambit (`in_game`), Vigi (`play`) and Keira's date are not expressible; cross-window rewrites are exact only for the chosen template's own window | Some answer combinations are refused or tell a different story | engineering + content design | 2-3 d | none | Yes |
| 11 | Item names in Russian in the start kit (4 of 42; the item-name key hash is unknown) (brief) | Russian players see raw ids for some items | offline research | 0.5 d | none | Yes |
| 12 | Steam Cloud: exported files and moved test saves may be synced or restored (PLAYER_GUIDE warns); behaviour with a new `ManualSave_` file was never examined | A silently reverted or duplicated save | in-game test | 30 min | none | No |
| 13 | Residual format risks, offline-proven but not or once in game: `JS_Failed` / appended NM names for journal statuses (gate M proved a CName only), remapped struct headers (`Vector`, `SActionPointId`) and `ActivatedInputs`, longer play after any transplant, meaning of `c1` and record `time`, `gameProp > gameplayFacts` mirror, Load-list ordering vs our UTC stamp, `runtimeGUIDCounter` | Could break a rare quest or sort saves oddly | in-game test + offline | 1-2 d | none | Partly |
| 14 | In-game runner mod (`C:\w3-out\auto_runner`, `modW3ALDev`): written, never compiled or launched; no WitcherScript compiler here | Would turn items 1, 2, 7, 12, 13 into unattended tests | engineering | 2-4 d | a script compiler, a dedicated saves folder | Yes once built |
| 15 | App click-through by a tester (installer builds; no-effect export is payload-identical); Phase 2 exit test is still partial | Nobody has used the UI end to end | UI test | 1 h | a tester | No |
| 16 | Patches: 4.04 support is neither decided nor tested (old-gen layout partly differs: 11 of 16 old saves fail the facts parse offline); a future patch bump of `saveVersion` is unknowable now (plan unknown 5) | Players on other builds are unsupported | offline research (4.04 only) | 1-3 d for 4.04 | decision | 4.04 yes, future no |
| 17 | Walker leftovers: `through` walk passes fact waits the donor's player had not reached; `sq102_hide_community` ends at 2 not 0; `sq102_explode_building` expiry not modelled; 5.01-only listener `8752d51c` stopped; no-date Keira (D3 `Out_no_date` not wired); no per-option question conditions in the schema (D15 / D18 contradictory answers make export refuse) | Edge-case mismatches against played saves | engineering | 1-2 d | none | Yes |
| 18 | Release: top-level license files, repository URL, README install steps, CI, Linux AppImage, `insta` / fuzz tests, `docs/playtests.md` (DECISIONS.md "Repo") | Needed to publish | engineering | 1-2 d | name decision | Yes |

Checklist counts after this triage: of the 18 unticked items, 9 are now ticked, 8 are partly answered (left unticked, remainder noted), 1 is
still open (future patch bump). Dated entries are untouched; their "open" lines are covered by the table (items 1-3, 5, 6, 9, 13, 17) or were
answered later in the same log (quest-state question: gates J, G-M; fact-index rebuild: gates A, B; `gameplayFacts` mirror stays item 13).

## Content research (Phase 3+)

- Fact names come from `content/content0/scripts` (`FactsQuerySum` / `FactsSet`), quest graphs (CR2W via WolvenKit/REDkit) and save diffs. Never from memory.
- Choice Impact (Nexus 14147) maps 7,085 choice lines to facts. Contact its author and check its license before using any of its data.
- Sample questbook values to verify: recommended levels (1, 1, 6), the White Orchard demo choice (a schema demo, not a real decision), the real Family Matters decisions and prerequisite chain.

## Findings

### 2026-11-04: first Phase 0 run (17 saves, 2016-2026)

Fixtures: 17 `.sav` copies in `C:\w3-fixtures` (outside the repo). Only **one** is from 5.x:
`ManualSave_52586_7ea4c400_1030937` (2026-10-18, build `5.0.1048522`). The other 16 are older
game versions (header ints `[64, 18, 163]` and similar) and only exercise the container layer.
(Superseded the same day by the Nexus downloads below: 55 more `saveVersion` 67 saves.)

Confirmed on all 17 saves (container):
- `SNFHFZLC`, data start 3,084, 12-byte records, raw LZ4 chunks; 4-13 chunks (the 5.x save has 5, 4.65 MB decoded, 1.16 MB on disk).
- **End offset is absolute** (`data_start` + cumulative compressed size); no `check-save` warning on any save.
- No 5.0.0c damage signature on any save.
- Decode -> re-encode -> decode gives an identical payload. Our compressor's output is about 2% smaller than the game's (1,140,464 vs 1,164,103 bytes).

Confirmed on all 17 (payload map): `SAV3` magic; footer = u32 variable-table offset + `SE`;
offsets are absolute (include the 3,084-byte header); the table is `u32 count` + (u32 offset,
u32 size). The `RB` section parses on every save (33-34 entries); it is `2 + 4 + n*6` bytes and
sits **immediately before** `NM`.

Corrections to the plan:
- **Header ints on 5.01 are `[67, 29, 164]`**, not 66/29/0. The `.json` sidecar confirms the order: `saveVersion` 67, `gameVersion` 29 (`buildPatch` is 0 there, so the third header int, 164, is something else; it grows 162 -> 163 -> 164 across game generations).
- **Variable-table entries are not in physical order and they nest** (a parent's span contains its children, e.g. rel 16..48 contains 20..48). The size field is real, but spans form a tree; top-level spans tile the payload. `Document::parse` now handles this.
- 5.x variable count was 75,566 (plan: 76,000-81,000). Older saves: 69,119-194,166.
- Tag histogram on the 5.x save: `VL` 52,061 · `BS` 16,275 · `OP` 1,702 · `SS` 150 · `MANU` 1 (plus 4,217 untagged).

`NM`: the layout is `NM` + `MANU` + body, and the body ends exactly at the 10-byte pointer block
before the variable table (the last table entry is the `MANU` token, 92,671 bytes on the 5.x
save). Our guess for the *body* (u32 count + packed strings) is **wrong**: packed-string reads fail with
"lone surrogate" on all 17 saves. Next step: look at the first bytes of the `MANU` body
(and Atvaark's `VariableParsers/`, MIT).

The 10 bytes before the variable table are `u32 NM offset`, `u32 RB offset`, then two bytes
`0x53 0x43` ("SC") on every save. Meaning unknown.

Sidecars (5.x): `.sav` + `.png` (63 KB) + `.json` (423 bytes, no signature or checksum, so a byte-for-byte copy is
plausible). JSON content: `saveMetadata` with `initial buildID`, `current buildID`
(`5.0.1048522 P4CL: 13413172 ...`), `gameVersion`, `saveVersion`, `buildPatch`, `platform`, `modsMetadata`.
Older saves have only `.sav` + `.png`.

Not yet tested: whether the game **loads** `AnotherLife_*.sav` (check A in PHASE0.md; the
repacked file is in `C:\w3-out`). Unknown whether the Load list needs the `ManualSave_<a>_<b>_<c>` name pattern.

Still unproven: the typed tokenizer, facts, and any edit. Regions are raw, so the payload round trip
passing is expected and is **not** the gate.

### 2026-11-04 (later): 56 saves downloaded from Nexus Mods

Source (third-party, untrusted; kept outside the repo in `C:\w3-extract`, only `.sav`/`.json`/`.png`
were extracted, never mod scripts): Nexus mods 13170 (WestAard, End of White Orchard), 14752 / 13779
(lukwraith, byte-identical archives, 45 saves), 13843 (imz, "Ultimate Save", edited), 11324 (hosamnasr,
9 saves with 6 mods listed in `modsMetadata`). No uploader's "no mods" claim was trusted: lukwraith, imz
and WestAard sidecars all say `numMods: 0`.

Results, 56 of 56 saves:
- Container read, payload map (footer, variable table, `RB`) and parse -> serialize all succeed; `roundtrip` PASS on every one (still raw regions, so this is not the gate).
- **55 are `saveVersion` 67** (header ints `[67, 29, 164]`), matching each save's `.json`. **One (WestAard, 2026-10-01) is `[66, 29, 164]`**: the pre-5.01 5.0 format parses identically. The third int is 164 on all, so it is a game build constant, not a per-save value.
- Sizes: 0.14-4.5 MB on disk, 1-18 decoded chunks, 13k-215k variables. The plan's "5-6 chunks, 76-81k variables" only describes a mid-game save; early-game 5.01 saves have a single chunk and about 13-20k variables.
- 10 bytes before the variable table are `u32 NM`, `u32 RB`, then `53 43` ("SC") on all 56.
- **`NM`/`MANU` body still fails on all 56** with the same "lone surrogate" error, so the body layout guess is wrong everywhere, not save-specific.

Deviation from the container hypotheses (5 of 56 saves, all single-chunk, all from the lukwraith set):
- the last record's end offset is not `data_start` + cumulative size (it differs by +831, +776, -35,149 bytes), and
- the header padding between the chunk table and `data_start` is **not** all zeros.
Both are consistent with the game reusing a larger buffer or file, leaving stale bytes. Whether the game reads the end offset is unknown. Our writer recomputes it, preserves non-zero padding only when its length is unchanged, and `check-save` reports the mismatch as a warning. Treat "the end offset is absolute" as holding for 51/56, not as a rule.

Useful for the next step: lukwraith's set is 45 consecutive manual saves from one 5.01 playthrough (Kaer Morhen
tutorial through early White Orchard, per the folder names), so neighbouring saves are ready-made before/after
pairs for `w3al diff`. The accompanying Portuguese guide describes each save; it has not been extracted or read.

Still unproven: the typed tokenizer, the `MANU` body, facts, any edit, and whether the game loads a repacked save.

### 2026-11-04 (evening): NM/MANU name table decoded

Method: hex dump of the first 256 and last 128 bytes of the `NM` section on a 5.01 save, then a
throwaway decoder run over every fixture (118 `.sav` files = 17 own + 56 Nexus + 45 byte-identical duplicates
in `lukwraith-8oct`, i.e. 73 distinct saves; includes 16 older-format saves and the one `saveVersion` 66).
Atvaark's `ManuVariableParser` (MIT) gave the shape; the bytes decided the details.

Layout, from the `NM` offset to the 10-byte pointer block at table offset - 10:

| Bytes | Meaning |
| --- | --- |
| `4e 4d` `4d 41 4e 55` | `NM`, then the `MANU` token (the last variable-table entry; it starts at NM + 2) |
| u32 | name count (5,554 on the 5.01 save in `C:\w3-fixtures`) |
| u32 | unknown; 0 on all 73 |
| count x (u8 len, len bytes) | names, no terminator, **plain one-byte length** (not a packed string: no ANSI bit, never a continuation bit) |
| u32 | unknown; 0 on all 73 |
| `45 4e 4f 44` | `ENOD` ("DONE" reversed) |

Evidence:
- The decoder consumes the section exactly (zero leftover bytes) on 118 of 118 files. The old guess (u32 count + packed strings) failed because the second u32 was read as the first string and because lengths have no ANSI flag.
- Max name length 58; empty names 0; no duplicate names in a table. No length byte ever had bit 6 or 7 set.
- Encoding: 2 names (in one save family) contain a byte >= 0x80 (`Azurite\xa0mineral`, `gelens\xe9r_farmstead_ft`): not valid UTF-8, so Latin-1 (cp1252-compatible). We decode as Latin-1, same as ANSI packed strings.
- Content (17,860 names on an early-game save, 6,775 on another): schema vocabulary first (`Entity`, `nP`, `Uint32`, `lockedByKey`, `Bool`, `CName`, `array:2,0,SBaseStat`, `handle:W3AbilityManager`, `W3PlayerAbilityManager`, `CWitcherGameResource`, `saveInfo`, `magic_number`, `runtimeGUIDCounter`, `facts`) then game identifiers (`Door_Closed`, `sq310_sewer_door_key`, `q104_fogling_blizzard`, `mq1043_final`, `HorseAxiiBuff`, `Light armor 09r`, `scabbard_steel_novigradian_04`). The table is in order of first use while the game wrote the save.
- Probable use: variables start with a 2-letter tag followed by **1-based u16 indices (name, type)** into this table. On an early save the pairs read `time:Uint64`, `type:Uint8`, `v:Uint16`, `path:String`, `count:Uint32`, `runtimeGUIDCounter:Uint64` with `index - 1`; with 0-based they read as nonsense. 4,579,204 of 4,579,986 `BS/VL/SS/OP` variables have a first u16 below the name count (the rest are not that layout or are mis-tagged, a tokenizer job). Not yet proven on every token type.
- The 10 bytes before the table are still `u32 NM`, `u32 RB`, `SC`. `RB` is `6 + 6n` bytes and ends exactly at the NM offset (checked on two saves: 3,956,281 + 210 = 3,956,491).

Open: meaning of the two zero u32; whether fact ids are in this table (facts are stored as packed strings in values, not looked up here, as far as seen); whether a save with more than 255-char names exists (a u8 length cannot hold it, so none is expected; the 5.0.0c bug would break the layout rather than lengthen a name).

### 2026-11-04 (night): facts blob located, parsed and re-serialized (PHASE0 step 8, part 1)

Method: a research pass on the lukwraith set found the `SXAP`/`SBDF` structure; `w3save::facts` now implements it
and `crates/w3save/tests/facts_fixtures.rs` proves it on every fixture (`W3AL_FIXTURES`). Reference tools
(throwaway, not in the repo) were read for the idea only; the code was written against the bytes.

Layout (all offsets inside the decoded payload; `SS` is a variable-table entry, `BS` is its only ancestor):

| Bytes | Meaning |
| --- | --- |
| `BS` u16 name | parent node (variable-table size = 4 + the whole `SS` variable; no length of its own) |
| `SS` u32 L | `L` = length of everything after it; always equals the table size - 6 |
| `SXAP` i32 i32 i32 | `67, 29, 164` on 5.01; `66, 29, 164` on the 5.0 save (same as the payload header ints) |
| `SBDF` u32 N | fact count |
| N x fact | packed string name, u16 `c1`, u16 `n`, `n` x 10-byte record |
| record | u16 value, f32 time, f32 expiry |
| `EBDF` | last 4 bytes of the `SS` body; nothing follows |

Real bytes, `bugfix_132404` (name 13 chars, `c1` 0, one record): `8d 62 75 67 66 69 78 5f 31 33 32 34 30 34 | 00 00 | 01 00 |
01 00 | 8d 46 8d 44 | 00 00 80 bf` = value 1, time 1130.2047, expiry -1.0. On the first save the variable starts `SS 2e 10 00 00` (L = 4,142).

Results (`parse` -> `to_bytes` byte-exact on the `SS` body, `to_ss_variable` equal to the original variable, `SBDF` count == facts parsed, no
duplicate names, `check_ids` quiet):

| Folder | Saves | Facts | Result |
| --- | --- | --- | --- |
| lukwraith-26oct (5.01) | 45 | 57,955 total (134 on the first, 1,992 on the last) | 45/45 |
| hosamnasr-allinone (5.01, mods listed) | 9 | 123,153 total (2,499 .. 25,189 per save) | 9/9 |
| whiteorchard-westaard (5.0, `saveVersion` 66) | 1 | 1,452 | 1/1 |
| imz-ultimate (5.01, edited) | 1 | 33,954 | 1/1 |
| `C:\w3-fixtures` (own saves, 2016-2026) | 17 | | 6/17: the 5.01 save and 5 old ones |

Old game versions (`C:\w3-fixtures`, header ints `[64, 18..27, 163]`): the **frame is the same** (`SS`, `SXAP 64 18 163`, `SBDF`, `EBDF` at the end,
nested in one `BS`) and 5 of 16 old saves parse completely. On the other 11 the declared fact count is larger than the number of parseable facts
(for example 515 of 2,021 or 1,644 of 1,666), and the next entry starts with bytes `6f 07 ...`, which is not a packed string; the data up to `EBDF` is
another structure (about 70 KB on the 2,021 save). Not forced: `w3al facts` reports the error. Old saves are out of scope (patch 5.01 only); this only records that the
5.x layout does not carry over to every old version.

What `c1` means (inferred, never from a script): the number of records with `expiry >= 0` (a real expiry time). It matched on 57,955 of 57,955 facts in the 45 lukwraith
saves and on every fact of the other 11 saves; permanent (`-1`) and dialogue (`-2`) records never share a fact with timed ones. Timed facts are the combat and tutorial timers
(`*_weapon_hit_*`, `tut_dialog_started`, `ach_*`, `q504_inn_door_opened`). The code keeps `c1` as a raw field (round trips even when it breaks the rule) and
only recomputes it on edit.

What the game seems to do (from 44 consecutive pairs of lukwraith saves; evidence, not scripts):
- **Add** appends a record and keeps the old ones: `q001_gwint_game_won` 1 -> 2 records, `player_had_quen` grows by many. The script API is a sum, so a fact is a list of events.
- **Set** replaces the list with one record carrying the new value and a new time: `CurrentlySelectedSign [v=1 t=1546.2 x=-1] -> [v=2 t=1845.0 x=-1]`.
- **First write**: a new fact appears with a permanent record `[v=1 t=<now> x=-1]` (`q001_vesemir_nightmare_talk`), or with zero records (533 times: `ach_attack`, created before anything is added). Facts are also left behind with zero records (1,021 times) when timed records lapse.
- **Order is hash order**: new facts show up at arbitrary positions and the survivors keep their relative order in only 26 of 44 pairs, so position should not matter (not tested in game).
- `time` is game seconds (about 1,100 on the first save, 41,000 on the last), `expiry` is an absolute game time (`time + 1..15` on timed facts).
- Value is u16 on disk; 118 of about 140,000 records hold 65535 (`q001_fistfight_ended`, `tut_fight_use_slomo`), so signed 16-bit is likely. Unproven.

Added API (`w3save::facts`): `FactTable::{parse, to_bytes, to_ss_variable, get, facts, len, set_value, add_record, check_ids}`, `find_facts_blob`, `parse_payload`.
Edits are pure functions on the table; the wiring into `Document::apply` followed the same night once `rebase` existed (see "RB decoded" below).

Open for the edit step:
- Does the game care about `c1` or about stale `time` values (a record stamped in the future of the save's own clock)? Which `time` should an exported fact carry?
- Is a fact with a duplicate-free but re-ordered table fine (appending vs hash order)? Does the game rebuild its index on load?
- Is the value field i16 or u16, and is the sum taken over `value` only or also over expiry (lapsed records)?
- `FactsSet` semantics are inferred from two or three facts; check against scripts (`content0`) before relying on "replace all records".
- Do any other places mirror fact state (journal manager, `questSystem` blocks that cache a fact value)? Plan unknown 3 stays open.
- The 11 old saves whose count exceeds the parseable facts: what is the second structure?

### 2026-11-04 (night): typed tokenizer (PHASE0.md step 7)

Method: hex dumps and a throwaway table-walker on 3 early-game saves (lukwraith 01, `C:\w3-fixtures` 52586, hosamnasr 1),
then the library itself (`crates/w3save/src/token.rs`) run on every fixture. A shape mismatch falls back to a raw node, so a
wrong guess shows up as a count in `w3al dump`, never as a failed parse. Atvaark's `VariableParsers/` (MIT) gave the shapes;
the bytes decided.

Token layouts (little-endian; `name` and `type` are 1-based NM indices):

| Tag | Bytes | Evidence |
| --- | --- | --- |
| `BS` | `"BS"` u16 name, then its children **inline**: the nested table entries, which tile the rest of the span exactly | 700,189 `BS` on 101 files, 0 mismatches. (Atvaark's `BS` is a bare header; his stream is flat.) |
| `VL` | `"VL"` u16 name, u16 type, value | 2,473,123; table size = 6 + value width |
| `OP` | same as `VL` (properties of managers, handles, arrays of structs) | 64,630 |
| `SS` | `"SS"` u32 len, body; `len` = span size - 6 | 17,003, 0 mismatches. The body is a `SXAP` stream, not nested variables |
| `MANU` | the NM table (see the NM Findings); last table entry, starts at NM + 2 | 101 |
| `SXAP` | `"SXAP"` + 3 u32 = `67, 29, 164` on 5.01 (the `SAV3` header ints again) at the start of every non-empty `SS` body | every non-empty `SS` |
| `SBDF`..`EBDF` | facts `SS` only, see "facts blob" above: u32 count, facts, `EBDF` | typed in `token::FactsBlock`; byte-exact on 101 of 101 |
| `BLCK` | non-facts `SS` bodies: `"BLCK"` u16 name, u16 size, u16 0, then `AVAL`/`PORP` items covering `size` bytes. 406 of 409 sampled `SS` start with it; the item chain checks out on 327 of 406; 400 of 406 have 40 or more bytes after the block that are not understood | partial; left as blob |
| `AVAL` | `"AVAL"` u16 name, u16 type, u32 size, value[size] (Atvaark calls the u32 "unknown"; here it equals the value width, 4 for `Uint32`) | 2 samples; partial |
| `PORP` | `"PORP"` u16 name, u16 type, u32 size, value[size]; the next item started exactly after it | 1 sample; partial |
| untagged | table entries that point INTO a parent's value (a packed string or GUID in an array of structs); no tag, no header | 5.2% of entries; always nested in a `VL`/`OP`/untagged parent, never top-level |

`AVAL`, `PORP` and `BLCK` are never table entries (the `SS` that holds them has no nested entries).

Value widths (body sizes on two saves + Atvaark): `Bool` 1, `Uint8` 1, `Uint16` 2, `Uint32`/`Int32`/`Float` 4,
`Uint64`/`Double` 8, `CName` 2, `CGUID` 16, `String` packed, `EngineTime` 3, `GameTime` 11, `Vector` 35, `EulerAngles` 27,
`IdTag` 17, `EJournalStatus`/`EZoneName`/`EDifficultyMode`/`eGwintFaction` 2 (an NM index on the `EZoneName` samples),
`array:2,0,T` = u32 count + elements. Decoded in the tree: those scalars, `CName`, enums, GUID, canonical packed strings.
Everything else stays an opaque value (bytes kept, nested entries kept as children); `token_stats().opaque_types` is the
work list (top: `EngineTime`, `array:2,0,SQuestThreadSuspensionData`, `SActionPointId`, `IdTag`, `Vector`).

Coverage on 101 files (73 distinct saves, 3,431,822 table entries):

| Tag | entries | typed | raw | values decoded |
| --- | ---: | ---: | ---: | ---: |
| `BS` | 700,189 | 100% | 0 | n/a |
| `VL` | 2,473,123 | 100% | 0 | 91.7% |
| `OP` | 64,630 | 100% | 0 | 98.4% |
| `SS` | 17,003 | 100% | 0 | n/a (facts `SS` typed on 101 of 101) |
| `MANU` | 101 | 100% | 0 | n/a |
| untagged | 176,776 | 0% | 176,776 | n/a |
| **all** | 3,431,822 | **94.85%** | 176,776 | |

All 176,776 raw entries are untagged sub-values (150,179 "other", 24,947 of exactly 16 bytes (GUIDs?), 1,650 packed-string
shaped); typing them needs the layout of their parent struct. Not one tagged token fell back to raw. The 16 older-format saves
(`[64, 18, 163]`) round-trip too, at 92-95% typed.

Round trip: `Document::parse` -> `to_bytes` equals the decoded payload, and `Document::table()` (recomputed from the tree)
equals the on-disk variable table, on 101 of 101 files: `tests/tokens_fixtures.rs`, run with `W3AL_FIXTURES` set to
`C:\w3-extract` (all five sets), `C:\w3-extract\lukwraith-26oct` (45), `C:\w3-extract\hosamnasr-allinone` (9) and
`C:\w3-fixtures` (17). `w3al dump` prints the per-tag summary.

Where the data lives (5.01, top-level `BS` names in file order): `CWitcherGameResource`, `saveInfo`, `InitialBuildIDBlock`,
**`facts`** (one child: the facts `SS`), **`questSystem`** (`quest` > `questThread` > `questBlock`, GUID-keyed, `numQuests`),
`community`, `attitudes`, `parentGroups`, `actorAttitudes`, **`CJournalManager`** (`JActiveEntries`), `CWitcherJournalManager`,
`CR4GwintManager`, `StrayActors`, `CModHandlerSystem`, `SoundClues`, `ScentClues`, `FocusModeVisibility`, `CCommonMapManager`,
`loot_manager`, `CR4TutorialSystem`, `SoundSystem`, `NewGamePlus`, `worldInfo`, `AdditionalSaveInfo`, `AdditionalContent`,
`ContainerManagerSaveInfo` (one `SS` child), `TimeScale`, `TickManager`, **`universe`** (`Player`, `world`s; 2.9 MB on the late
save), `worldState`, **`gameProp`** (`zoneName`, `savedEnchanterFunds`, `gameplayFactsForRemoval`, and `gameplayFacts`: an
`SGameplayFact` array whose items are fact ids such as `PlayerIsGeralt`), `timeManger`, `idTagManager`, `envManager`; then `MANU`.
The only gaps between top-level variables are the 16 header bytes, `RB` + the `NM` tag (212 bytes on a 34-entry `RB`), and the
tail (pointer block, table, footer).

Design for `rebase` (step 8): every node reports `encoded_len()` and `Document::table()` already emits the table from the
tree. The `SS` length is derived when writing, so growing the facts `SS` needs no length fix-up. (The `RB` mapping and the
pointer block were resolved later that night, see "RB decoded" below.)

Open:
- `gameProp` > `gameplayFacts` also lists fact ids: runtime copy or extra state? Does an edit of the facts `SS` need a matching change there?
- What follows the `BLCK` in non-facts `SS` bodies; `AVAL`/`PORP` coverage; the 5.2% untagged sub-values.
- Whether `questSystem` / `CJournalManager` must change for a quest to show as done (plan unknown 3). Both are now typed `BS` subtrees.

### 2026-11-04 (late night): RB decoded, `rebase`, `apply`, gate saves (PHASE0 step 8 part 2)

**`RB` entries** (`"RB"`, u32 n, n x (u16, u32)). The u16 is NOT a size: it is the 1-based NM name index of a top-level `BS`
and the u32 is the absolute offset of that `BS`. Entry `i` is the `i`-th top-level `BS` in file order. Evidence: a throwaway
probe built the list (name, absolute start) of every top-level `BS` from the typed tree and compared it with `read_rb`; equal
on **118 of 118** files (17 own saves + 101 files in `C:\w3-extract`, 73 distinct saves, every game version seen, 33-34
entries each). Example (own 5.01 save): `facts` = (87, 13150), `questSystem` = (359, 152750), `universe` = (1753, 1038305),
`envManager` = (1793, 3956232). The "size" values rise monotonically because NM names are in order of first use. `MANU` is not
a `BS` and has no entry. `tests/rebase_fixtures.rs` re-asserts this on every file.

**Everything that holds an absolute offset** (all rewritten from the tree by `Document::rebase`):
the variable table (offset, size of every node; parents of the edited node grow, everything behind it shifts; count never
changes), the `RB` offsets, the pointer block `u32 NM offset` (offset of the `NM` tag) + `u32 RB offset` (offset of the `RB` tag) +
`SC`, and the footer `u32 variable-table offset` + `SE` (the table starts 10 bytes behind the `MANU` token). The `SS` length is
derived on write. Nothing else tiles with an offset: the 16-byte header has no length or pointer.
Scan for hidden pointers: for the 118 files, every u32 equal to the absolute or relative start/end of a top-level block, or to
the payload/file length, outside the `RB` section and the tail was counted. 206 chance hits in 101 files, almost all values in
the 190k-630k range sitting once or twice in the middle of unrelated data (one save repeats one value 98 times at a fixed
stride, i.e. a data field, not a pointer); 3 in 17 own saves. No evidence of internal pointers, but this is an absence-of-evidence
check; only the in-game load can prove it.

**Rebase tests** (`tests/rebase_fixtures.rs`, `W3AL_FIXTURES`): on all 118 files the unedited document rebases to the identical
payload. On 6 of the 17 own saves (the 5.01 one plus 5 old ones that have facts) and 9 of the 101 Nexus files, the facts blob was
grown by 1 fact, 500 facts, 5,000 long-named facts, and shrunk by 3 facts: after each, `Document::parse` + `payload::read_map`
of the output show every table entry unchanged/shifted/grown exactly as predicted (entries behind the facts block shift by the
delta, the facts `SS` and its `BS` ancestor grow by it, entries in front do not move), every `RB` name unchanged and offset
shifted likewise, NM/RB pointers and footer shifted, names identical, only the `facts` BS and the two gaps (RB, tail) differ
from the original tree, and the container writes and reads back to the same payload.

**`Document::apply` for facts**: each `(name, value)` is applied with `FactTable::set_value` (FactsSet semantics): the fact ends up
with ONE permanent record `{value (i16 bits), time, expiry -1}`, `c1 = 0`, appended at the end of the table when new.
`time` = `FactTable::latest_time()` (the newest record time in the table, e.g. 43977.957 on the own save). All of these are
HYPOTHESES not yet tested in game: (1) "set replaces the list" (inferred from save diffs; scripts not read), (2) that a record
stamped with the newest existing record time is acceptable (the true clock, `timeManger` > `time:GameTime`, is an undecoded
opaque value), (3) that table position and record order do not matter, (4) that values are i16 (-32768..65535 accepted).
`FactTable` got the exact-state helpers `set_records` (list, `c1` by the inferred rule), `set_fact` (verbatim incl. `c1`),
`remove`, `differing`, `match_fact`, `latest_time`. Quest and journal edits still return `Error::Unverified`.

**Gate saves** (`C:\w3-out\gate`, README.txt there). A: own 5.01 save + `w3al_gate_a_test_fact` = 1 (+36 bytes). B: lukwraith 06
with the six facts that differ in the 05 -> 06 pair restored to 05's exact state (`q001_gwint_game_won` 2 -> 1 record,
`tutorial_interaction_talk` re-added, `83805989_2_1` removed, three timer facts restored); `w3al facts 05 --diff B` is empty, payload -7 bytes.
C: unedited repack named `ManualSave_52586_7ea4cc00_10309b7`. All three: `roundtrip` PASS, `check-save` clean, `edit-fact` re-read VERIFIED.
In-game results pending (PHASE0.md step 9).

Open after this step:
- Whether the game accepts the rewritten offsets and the new fact (gate A), the exact restore/removal (gate B). Objective proof is
  the facts of the save the game writes afterwards (README.txt step 5).
- Load-list label: file-name ids vs payload vs sidecar (gate C).
- What the game does with `gameProp` > `gameplayFacts` when `facts` changes; `FactsSet` semantics against the scripts.

### 2026-11-04 (evening): first in-game results for edited saves (gate A/B/C)

Setup: three sets from `w3al` in `C:\w3-out\gate`, copied by the user into `gamesaves` (new names, nothing
overwritten). The user loaded each, played, and made in-game saves; copies of those are in
`C:\w3-fixtures\after_gate`. User-reported: A, B and C load and play fine, and the saves made from them load.

Objective re-read of the saves the GAME wrote afterwards (`w3al facts`, `check-save`, `roundtrip`):
- **A (new fact, facts block grew 36 bytes, variable table / RB / pointers / footer rebased):** the game's
  re-save (3224 facts) contains `w3al_gate_a_test_fact c1=0 [v=1 t=43978.0 x=-1.0]` byte-for-byte as we
  wrote it. So the game read our edited facts table and wrote it back; it did not rebuild or drop it.
  `check-save` clean, `roundtrip` PASS. This is evidence for: rebase offsets are right, a hand-appended fact
  with a permanent record is accepted, fact order/time need not be game-like.
- **B (lukwraith save 06 with the 05-state facts restored exactly):** the game's re-save keeps
  `q001_gwint_game_won` at ONE record `[v=1 t=2304.8]` (06 had two), and the user's play afterwards added the
  fist-fight facts (`q001_fistfight_*`, `tut_fist_fight_*`), so the restored table was live. Nothing
  visible was reported; the prologue facts have no obvious visible effect.
- **C (unedited repack, `ManualSave_` stem):** loads; its re-save is clean (3223 facts, no gate fact).
  The Load-list label question (quest name shown or not for the `ManualSave_` stem) was NOT answered yet.
- Game-written single-chunk saves (B's re-save and its CheckPoint) show the SAME end-offset deviation as
  five lukwraith saves (`end offset 339092` vs computed `3084 + 235047`). So the game itself writes a value
  that is not "data start + compressed size", and it loaded our files whose value we recompute. The
  "absolute end offset" hypothesis is therefore NOT something the game relies on for single-chunk saves;
  keep recomputing it. (Meaning of the stored number still unknown.)
- The first number in the file name (`a`) was 52586 for the files we exported, 52587 for the game's re-save
  of A, and unchanged (8559a) for B. Meaning unknown; no effect on loading seen.

Not proven yet: a VISIBLE in-game effect of an edited fact (the plan's gate wording), quest/journal edits,
whether `c1`/`time` are checked, position/spawn.

#### Load-list label (answers the sidecars-and-naming question, user-reported 2026-11-04)
- Gate C (unedited repack of the user's own save, named `ManualSave_52586_7ea4cc00_10309b7`) shows the QUEST NAME
  in the Load list. The first unedited repack, named `AnotherLife_20261104_172834` (same payload and sidecars
  copied the same way), showed NO quest name. Only the file name differs, so **the Load-list label needs the
  `ManualSave_<a>_<b>_<c>` naming (or at least that prefix/shape); the label is not stored in the payload
  or sidecar contents we copy.**
- Ids we invented (new `b` and `c`, `a` kept from the template) were accepted. Their real meaning is still
  unknown. Observed: within one playthrough `b` and `c` grow with every save (`b` in steps of 0x400 in our
  samples, `c` monotonic), so they look like counters/time stamps. Whether the game cares that they are
  unique, increasing or tied to the payload is untested; keep them unique and larger than the template's.
- Consequence for the product: the exporter's default output name must switch from `AnotherLife_<timestamp>`
  to a `ManualSave_<a>_<b>_<c>` name (keep `a` from the template, bump `b`/`c`, never clash with an existing
  file). DONE 2026-11-04: `export::default_stem`; what `a`/`b`/`c` are is in `docs/findings/save-name-ids.md`
  (`b`/`c` = packed local date and time of the save, `a` = tracked-quest key).

### 2026-11-04 (night): script semantics of the facts API, and a visible-effect fact (gate D)

Method: read-only pass over the game's own plain-text scripts, `...\The Witcher 3\content\content0\scripts\game` (no CDPR file
copied; one-line quotes only). `FactsAdd`, `FactsQuerySum`, `FactsQuerySumSince`, `FactsQueryLatestValue`, `FactsDoesExist` and
`FactsRemove` are `import function` (native code, no body in the scripts); only the wrappers are script.

What the scripts prove (`game\facts.ws`):
- `FactsSet(ID, val, validFor)` (lines 32-41) is `FactsRemove(ID);` then `if(validFor == 0) validFor = -1;` then `FactsAdd(ID, val, validFor);`.
  So after a set the fact holds exactly ONE record, whatever it held before, and it is permanent (expiry -1, `c1 = 0`).
  This proves the "set replaces the record list" hypothesis and `Document::apply` / `FactTable::set_value` match it. The only
  difference is the table position (the engine removes and re-adds; we replace in place), and position is hash order in real saves.
- `FactsSubstract(ID, val)` (lines 43-50) is `FactsSet(ID, Max(0, FactsQuerySum(ID) - val))`: a set, so the result is again one record,
  possibly a real `v=0` record (such records exist: `CurrentlySelectedSign [v=0 ...]`).
- `FactsAdd(ID, value, validFor)` APPENDS a record (`ResetFactQuest`, `quest_function.ws:3070-3076`, is `FactsAdd(factID, -FactsQuerySum(factID))`,
  i.e. the sum is cancelled by a negative record, not by deleting records). `FactsQuerySum` is therefore a sum over records.
  `validFor` is in game seconds: `FactsAdd("ach_counter", 1, 4)` (`actor.ws:5471`) matches the timed records seen in saves
  (`expiry = time + 4`, e.g. the real `ach_attack` bytes in the unit tests); `-1` is permanent. A plain `FactsAdd(ID)`
  (`tutorialManager.ws:1017`, `tutorial_interaction_talk`) leaves `[v=1 x=-1]` in the saves, so the native default is value 1 and permanent.
- Values are SIGNED: `FactsAdd(factOnPositionValid, -1, -1)` (`riddleNode.ws:298`) and `FactsAdd(factID, -sum)`. The disk field
  is 16 bits (10-byte records), the script type is `int`; 65535 = -1 on disk (118 records) is consistent, values outside
  `-32768..=65535` cannot be stored. Reading the field as i16 stays a HYPOTHESIS only in the sense that no script line states the width.
- `FactsQuerySumSince` is declared but never called in the scripts (native quest conditions may use it), so the record `time` only
  matters to native code. `FactsQueryLatestValue` is used for option-like facts (`gwent_difficulty`, `altcast_before_switchtutorial`).
- Not answerable from scripts: how the engine stamps `time` (we use the newest record time of the table, a lower bound of the clock), what
  `c1` is for, whether `FactsDoesExist` is true for a fact with zero records (the game leaves 1,000+ of those in saves), whether
  `FactsQueryLatestValue` picks by time or by list position. Comparison with our layout: record `{u16 value, f32 time, f32 expiry}`,
  `c1` = number of records with expiry >= 0, all consistent; no change to `apply` was needed.

Tutorial hints are NOT facts-only: `DisplayTutorialHint` (`tutorialManager.ws:611-617`) skips a hint when `HasSeenTutorial(tag)` (journal
state, `CJournalTutorial`); the facts `factOnFinishedDisplay` are only an output of that. So reverting a tutorial fact does not re-fire
the popup. Likewise a quest-state revert (lukwraith 21 -> 22 `mq0005_complete`, 38 -> 40 `q002_met_emhyr`) would only be visible through
`questSystem` / `CJournalManager`, which store their own state; those are the unverified parts (plan unknown 3), so they were not used.

Chosen visible fact: **`gwent_difficulty`** (a pure `FactsQueryLatestValue` read, no other storage):
- `gui\main_menu\ingamemenu\igmOptions.ws:1281-1289` (`IngameMenu_AddGwentDifficultyOption`): `difficulty = FactsQueryLatestValue('gwent_difficulty');
  if (difficulty != 0) difficulty -= 1;` and that number is the selected entry of the list `[easy, normal, hard]` of the pause-menu
  Options > Gameplay page (the `else` branch at `igmOptions.ws:265-275`, i.e. in-game menu only, not the main menu). So 1 = Easy, 2 = Normal, 3 = Hard.
- `ingameMenu.ws:2137-2145` writes it with `FactsSet('gwent_difficulty', 1|2|3)` from that menu; the own save already holds such a record
  written by the game: `gwent_difficulty c1=0 [v=1 t=22079.6 x=-1.0]` (one permanent record, same shape as `set_value`'s).
- `r4Player.ws:707-708` in `OnSpawned` (runs on every load, not inside the `restored` branch): `if ( !FactsDoesExist("gwent_difficulty") ) FactsAdd("gwent_difficulty", 2);`.
  So a table WITHOUT the fact must come back as Normal after load, and the re-saved file must contain a new record.
- `gwintManager.ws:323-339` also uses it for the AI (+80 / 0 / -80 balance), a second, invisible effect.
- Safe: no quest, no softlock, reversible in the same menu.

Gate D sets (`C:\w3-out\gate_D`, README.txt there), both built from the own save `ManualSave_52586_7ea4c400_1030937` (which holds Easy):
D1 `w3al edit-fact --set gwent_difficulty=3` (apply: one permanent record `v=3 t=43978.0`, nothing else changed; expected Hard), D2 `--remove
gwent_difficulty` (new `--remove` option of `edit-fact`; expected Normal after the game's own `FactsAdd(.., 2)`). Both: `roundtrip` PASS, `check-save` clean,
`facts --diff` shows exactly that one fact. In-game results pending.

What could still make it inconclusive: the pause-menu list is read when the menu opens (not cached) as far as the script shows, but the Flash side is
not visible to us; if D1 shows Easy the table was not the source (or `FactsQueryLatestValue` is not what the UI shows) and the facts section would not be
live, which A and B already contradict. Facts that live in native-only logic (`c1`, time) are not exercised by a menu read.

### 2026-11-04 (night): gate D result and Phase 0 verdict

User-reported in game, from `C:\w3-out\gate_D` (both built from the user's own 5.01 Novigrad save):
- **D1** (`gwent_difficulty` set to 3 through `Document::apply`): pause menu, Options > Gameplay shows **Hard**.
- **D2** (`gwent_difficulty` removed): shows **Normal** (the game re-adds it with value 2 on spawn, per `r4Player.ws`).
- The original save shows **Easy** (value 1). So an edited fact visibly changes the game, in both directions, and
  our `FactsSet` semantics (remove, then add one permanent record) match what the game reads.
- The user loaded D1 and D2 only to read the menu; no in-game re-saves of D1/D2 were made, so the objective
  facts re-read for gate D was not done (it was done for A and B, see above).

Phase 0 criteria (PHASE0.md step 9), as met:
1. Typed round trip identical on 5+ saves from different points: 101 files / 73 distinct saves, plus rebase no-op on 118.
2. Edited save listed, loads, world shows the change: A, B, D1, D2 load; D1/D2 show the change in the menu.
3. A new in-game save made from an edited one loads and `check-save` is clean: A's re-save (`ManualSave_52587_7ea50c00_5a639fc`) is clean and loads (user-reported).
Verdict: **gate passed for fact edits.** Not covered: quest/journal state, position, world/DLC/mod state,
other game patches. The next risk is whether facts alone are enough to show quests as done (see
`docs/findings/quest-state.md` when the research agent finishes).

### 2026-11-04 (late): position and world-swap tests (user-reported in game)
Sets from `docs/findings/position-world.md`, loaded one at a time:
- **T1** (lukwraith 04, same world, position and yaw overwritten with the garrison spot): loads with Geralt at the garrison; a cutscene
  started on load (that spot's quest scene triggered). So a same-world position edit works, and scripts react to the new location.
- **T2a** (White Orchard save moved to Kaer Morhen: `worldInfo > world`, `LoadingScreenName`, player entity moved
  between `layerStorage` blocks, new position): the player is in Kaer Morhen.
- **T2b** (the user's Novigrad save moved to White Orchard, 60 stale prologue entities left in the target block): the
  player is in White Orchard.
Not yet checked: whether the saves play normally for longer, whether the game's own re-save of T2 keeps the world, stale
entity effects, z-snap behaviour, quest-scene triggers from a teleport (T1's cutscene shows they fire).
Consequence for the design: a world swap into a world the template has already visited works, so one player-owned
template can be re-homed; a never-visited world still needs a stub or a template (untested).

### 2026-11-04 (late): journal editor prototype and gate J saves (plan unknown 3)

Design and API: `docs/findings/journal-editor-design.md`. Code: `w3save::journal`, `w3al-core::journal_effect`, CLI `w3al journal`,
`journal-diff`, `journal-set`. Tests: `crates/w3save/tests/journal_fixtures.rs` (plus a post-order assertion in `tokens_fixtures.rs`).

Proven offline (tests, `W3AL_FIXTURES` = `C:\w3-extract` and `C:\w3-fixtures`):
- **Variable-table index = post-order rank** of the node (children first) on 122 of 122 files, every game version. So nodes can be added:
  renumber in post-order, `rebase` writes the longer table (`Document::renumber_post_order`; `rebase` no longer requires a fixed count).
- Strict journal layout on 101 of 101 5.x saves (58,632 entries): path depth 1..5 (5 only late-game, 181 entries), `flags` 0 on inner steps and
  1 on the last on every entry, no duplicate GUID paths, `Size` == count. Statuses seen: `JS_Active` 24,679, `JS_Success` 33,418, `JS_Inactive`
  422, `JS_Failed` 113 (never on a quest-level entry).
- 44 consecutive lukwraith pairs: entries are append-only, in order; replaying `diff(A, B)` on A gives B's decoded quest log on 44 of 44 (counters compared as a set in the 4 re-hash pairs) and
  B's `CJournalManager` bytes on 43 of 44 (01 -> 02 needs `JS_Success` added to the NM table; same name, different index).
- **lukwraith 21 -> 22: the edited 21 has byte-identical `CJournalManager` (44,979 B) and `CWitcherJournalManager` (1,950 B) to the game's own 22**;
  5 flips, 6 appends, Size 208 -> 214, +1 counter, tracked quest and highlighted objective retargeted, every other block unchanged.
- `JObjectiveCounters` are a hash-ordered map: the new mq0005 counter is at slot 27 of 45, old counters keep their order except in 4 of 44 pairs
  (re-hash). Editor inserts at the donor position (`at`) or appends.
- **The game's NM table is append-only between saves** (43 of 44 pairs: B's names = A's + new names at the end). Appending a missing name is
  therefore game-like and changes no index (`Document::intern_name`, opt-in `--allow-new-names`; untested in game).
- `CWitcherJournalManager` side tables (`JHuntingClues`, `JMonsterKnown`, `JEntryAdvancedInfo`) change in 7 of 44 pairs; not modelled, reported by
  `journal-diff`.

Gate J (`C:\w3-out\gate_J\README.txt`; template lukwraith 21, `ManualSave_fd3e1_7ea4d800_3f4cf08`, facing Napp):
- J0 `ManualSave_8559a_7ea4d800_4038706` = real save 22, byte copy (control). J1 `ManualSave_8559a_7ea50c00_4df4d4d` = 21 + only the 21 -> 22 journal
  changes, built through the TOML path (`journal-diff` -> TOML -> `journal-set --from-toml`). J2 `ManualSave_8559a_7ea50c00_4df7d86` = J1 + six facts
  (`mq0005_arsonist_gets_up`, `_axiid_arsonist`, `_brought_arsonist`, `_complete`, `_stop_hypnosis`, `prologue_smith_as_community` = 1).
  `a` = `8559a` because the tracked quest is now `7c198a76` (as in 22). Thumbnails stamped TEST J0/J1/J2.
- All three: `roundtrip` PASS, `check-save` clean; `journal-diff J1 22` and `J2 22` empty; `facts 21 --diff J1` empty, `--diff J2` exactly the 6 facts;
  J1/J2 `facts`, `questSystem`, `community`, `universe`, `gameProp` blocks byte-identical to 21.
- In-game results pending. Questions it answers: does the quest log come from `CJournalManager` as stored (quest shown completed)? Does the
  still-waiting quest graph re-trigger the Napp scene or re-open the quest? Are appended entries, the new counter and the renumbered table accepted?

Open: quest-level `JS_Failed`; appending a whole never-started quest (Case B); whether the game accepts an NM name we appended; whether the
counter order matters; the stale `questSystem` frontier after a completed-by-edit quest.

### 2026-11-04 (night): gate J results for the journal editor (user-reported + our re-read)
- **J0** (real save 22): shows the arsonist quest completed (control OK).
- **J1** (save 21 + journal changes of 21->22, no facts): the Journal tab shows the quest COMPLETED on load. So the game takes the
  displayed quest status from `CJournalManager` as stored; flipped statuses, 6 appended entries, the new counter and the
  renumbered tables all loaded without a crash. (The user then stood next to Napp, who was still in the world as in save 21.)
- **But the stale quest graph reopens it:** talking to Napp (the world and `questSystem` were still save 21's: waiting at the
  Napp decision) flipped the quest from Completed back to Active (the game rewrote 3 journal entries Success -> Active and
  appended 2 objective entries); it could not be finished before talking to Napp. The user then played it out, spared or led
  Napp to the smith, and finished it with Willis.
- Re-read of the user's three saves from J1 (`C:\w3-fixtures\after_gate_J`, all `check-save` clean):
  1. spared Napp (`ManualSave_fd3e1_..._5f09934`): 3 entries Success -> Active, 2 entries appended (objective under `3eb16b1a` and `f02a0417/4d995e13`), counter +1, facts `24353285_2_1`, `mq0005_arsonist_gets_up`;
  2. did not spare, lead him to the smith (`..._5f21d8c`): 3 entries Success -> Active (incl. objective `753019eb`), facts `24353285_2_0`, `24353285_5_1`, `mq0005_axiid_arsonist`, `mq0005_brought_arsonist`;
  3. finished after talking to Willis (`ManualSave_8559a_..._5f2c555`): the quest log is IDENTICAL to J1 (`journal-diff` empty); facts add `mq0005_stop_hypnosis`, `mq0005_complete`, `99245509_2_1`.
  So our edited quest log equals the game's real end state; the game ends in J1's journal after finishing the quest for real.
- **Design consequence (important):** the quest GRAPH (`questSystem`) is the authority for what can happen next; journal + facts only
  decide what is SHOWN. If a template's graph is still waiting at a quest's decision, interacting with the NPC re-activates the
  quest. So for a completed quest the template's graph must already be past that quest (capture act templates AFTER the quests
  that should count as done, or patch the graph, which we cannot do yet). Branch choices (spare / not spare) are graph state
  plus dialogue-id facts (`24353285_*`); facts alone do not steer the graph. J2 (journal + facts) is being tested for whether
  the facts stop the scene.

#### Gate J2 result (journal + 6 quest facts), user-reported + re-read (`C:\w3-fixtures\after_gate_J2`)
- Load: quest shown completed. A save made right after loading (`ManualSave_8559a_..._5f5a072`) has a journal identical to J2 and only timer-level fact changes.
- Talking to Napp: the quest became ACTIVE again, exactly like J1 (`ManualSave_fd3e1_..._5f6a577`: the same 3 entries Success -> Active, the same 2 appended objective entries, counter +1, tracked quest = the dwarf-smith quest), plus the dialogue fact `24353285_2_1`; `mq0005_arsonist_gets_up` got a second record (the scene ran again).
- So **setting `mq0005_*` completion facts does NOT stop the Napp scene or the reactivation**: the stale `questSystem` graph (waiting at the decision) drives the NPC dialogue and rewrites the journal. Journal + facts only fix what is SHOWN at load. Confirmed: a quest counts as done in play only if the template's graph is already past it.

### 2026-11-05: the quest graph (`questSystem`), graph transplant, gate G saves

Design and evidence: `docs/findings/quest-graph-design.md`. Code: `w3save::graph`, CLI `w3al tree`, `w3al graph-diff [--toml K --scope P]`,
`w3al journal-set --graph-donor D --graph-op/--graph-from-toml`. Tests: `crates/w3save/tests/graph_fixtures.rs` + unit tests in `graph.rs`.

Proven offline (W3AL_FIXTURES = `C:\w3-extract`):
- `questSystem` = scene-player list (`ExternalDialog {tag, dialogsCount, guid}`) + 10 `quest` graphs of recursive `questThread`
  (scope wrapper `{GUID, inner thread}`) and `questBlock {GUID, inputName, activationState, conditions, ActivatedInputs}`: the live frontier.
  On 101 of 101 5.x saves (112,536 checks): `numBlocks`/`numThreads`/`tagsCount`/`numCommunities`/`numStubs` = number of such children, and every
  `ExternalDialog.guid` is a waiting `questBlock` GUID.
- Replaying the keyed diff of `questSystem` + `community` of a consecutive pair on A gives B's blocks byte for byte on 86 of 86 lukwraith pairs
  (06 -> 07 refused: NM table not a prefix).
- mq0005 (21 -> 22), confirmed by a second independent completion (the user's own play from J1, same branch): the quest's graph delta is its phase
  scope `38424c39` (Napp's scene thread `3446e000` with scene block `300f579b` and branch `62a65bce` leave; an epilogue wait remains), two listener
  blocks in other graph branches (`fd4ddac0` in `83ca614d`, waiting since save 03, fires and registers the smith's scene `f2bff115`; `080f5fdd`
  in `408d8753`, since save 08, leaves), and the scene list (-`mq0005_arsonist`, +`prologue_smith`). `community` loses the arsonist's spawn set and
  stub; stubs are removed by swap-remove (the last stub takes the slot).
- J2's own re-save shows that fact edits DO fire waiting condition blocks on load (`fd4ddac0`, `62a65bce`, `6f50331f`), but not the scene block that
  waits for the player (`3446e000`): that is why J1/J2 reopened the quest.
- Scoped transplant: the mq0005 ops on 21 make those nodes equal to 22's and change nothing else in the graph; other quests' scopes (24 -> 25,
  25 -> 26, 36 -> 37) each replay on their own. A quest is not one subtree (own scope + listeners elsewhere), and its scope may vanish entirely
  (mq0002, 24 -> 25) or remain with an epilogue (mq0005).
- Time fields in the graph: `conditions > timeRemaining` as `Float` (5 occurrences) or `GameTime` (7); not in the mq0005 scopes.
  Opaque values with name indices exist (`ActivatedInputs: array:2,0,CName`), so transplants require the donor's names to extend the template's.

Gate G (`C:\w3-out\gate_G\README.txt`, Russian; template lukwraith 21, donor 22):
- G0 `ManualSave_8559a_7ea4d800_4038706` = real save 22 (byte copy, control).
- G1 `ManualSave_8559a_7ea50c00_50f4f2b` = 21 + journal of 21 -> 22 + the six J2 facts + mq0005 graph ops (`deltas/mq0005_graph.toml`, 16 ops) +
  community ops (`deltas/mq0005_community.toml`: remove the arsonist's spawn set, swap-remove its stub, recount). So G1 = J2 + graph + community.
- G2 `ManualSave_8559a_7ea50c00_50f5a10` = the same journal and facts + the WHOLE `questSystem` and `community` diff of 21 -> 22 (106 ops).
- All three: `roundtrip` PASS, `check-save` clean, `journal-diff X 22` empty; `facts 21 --diff` = the six facts (G1, G2); `graph-diff G2 22`
  questSystem and community byte-identical; `graph-diff G1 22`: 0 changes under the transplanted paths (18 left in the four non-mq0005 scopes,
  plus the smith's spawn set and NPC runtime fields). `universe` and map pins stay 21's in G1/G2. Thumbnails stamped TEST G0/G1/G2.
- In-game results pending. Not marked proven: whether the game runs the transplanted frontier and whether Napp's scene stays gone.

#### Gate G result (2026-11-05), user-reported + re-read (`C:\w3-fixtures\after_gate_G`)
- **G0** (real save 22, control): quest shown completed; talking to Willis starts nothing; Napp is absent. Re-saves (`G0\ManualSave_8559a_7ea51000_1dcd89`,
  `_1fafb9`) are clean: no journal change except `JEntryUnread` cleared on the quest entry, no `mq0005_*` fact change.
- **G1** (save 21 + journal + six facts + the mq0005 graph and community ops only): **loads, no crash, quest shown completed, Napp is gone
  from the world, talking to Willis does not reopen the quest.** Re-saves (`G1\ManualSave_8559a_7ea51000_22a3b4` right after load,
  `_23ce2d` after talking to Willis) are `check-save` clean; journal unchanged except `JEntryUnread` of `minorquests/mq0005dwarfsmith`;
  no `mq0005_*` fact changes; the only new fact is the dialogue id `12721093_1_3` (also present in the real save 22: the normal post-quest talk
  with Willis); `graph-diff G1 <re-save>` shows no change in the transplanted scopes (`38424c39`, `83ca614d`, `408d8753`, the scene list) and no
  re-added arsonist spawn set or stub; the remaining graph changes are in unrelated scopes (`09366804`, `bob_quest`), the same ones that change
  on load in J2's re-save.
- Compare J1/J2 (same journal and facts, graph and community left as 21's): talking to Napp reopened the quest. So the difference is the graph +
  community transplant. **Proven in game for this one quest, one template, a donor from the same playthrough:** a quest counts as done in play when its
  graph scopes, listener blocks and scene-list entry are transplanted from a save past it, together with journal and facts. The NPC also vanished
  although `universe` (entity blobs) was still save 21's: removing the spawn set and stub was enough here.
- G2 (whole-block variant) was not tested; it was only needed if G1 failed.
- Not shown: other quests, branches the donor did not take, two deltas on one template, cross-playthrough donors, `JS_Failed`, and long play
  after G1 (only a few minutes near the forge).

### 2026-11-05 (later): gate P (Precious Cargo) passed; donor S captured; gates F and S built

- **Gate P** (`C:\w3-out\gate_P`; template lukwraith 24, donor 25; quest `mq0002thefield`, no completion fact): P1 = 24 + journal (7 flips,
  5 appends, 2 counters, tracking) + facts `mq0002_merchant_ran`, `mq0002_stop_riding` + graph (phase scope `3d284cad` leaves the hub
  `b4e73851` entirely; scene list replaced, `mq0002_merchant` gone) + community (spawn set `mq0002_merchant.w2comm` switched to phase
  `mq0002_merchant_at_camp`, merchant and horse stubs from 25). User-reported: P0 and P1 load, quest completed, no merchant at the swamp,
  no reopen. Re-saves (`C:\w3-fixtures\after_gate_P\P1`): journal unchanged except `JEntryUnread`, no `mq0002_*` fact change, the merchant's
  stub at the camp position of save 25 (387.6, 71.7). **Second quest proven, of the other shape** (scope removed, NPC kept and moved).
- Child order on removal: blocks are swap-removed (24 -> 25, 22 -> 23); the thread list is swap-removed in 22 -> 23 (hub and `8e15d30f`) but kept
  its order in 24 -> 25. Not explained; G1 (plain removes) and P1 worked, so order is probably not semantic. `graph-diff` flags it as `o`.
- **Donor S** (user-played from lukwraith 21, Napp spared, quest finished with Willis; `C:\w3-fixtures\donor_S`): a different branch in every
  layer: journal appends `3eb16b1a/82123313`, `f02a0417/4d995e13`, `3eb16b1a/cf5c3606` (brought-branch: `753019eb`, `2734d586`); facts only
  `mq0005_arsonist_gets_up`, `mq0005_complete`, dialogue `24353285_2_1`; graph: the same four scopes, but `ExternalDialog[mq0005_arsonist]` stays
  with a new scene `6453be42` waiting in epilogue thread `4f5f58fc`; community unchanged (Napp stays).
- Built, in-game pending: **S1** (`C:\w3-out\gate_S`) = 21 + donor S's journal, 2 facts and graph scopes; **F1** (`C:\w3-out\gate_F`) = 22 +
  mq0004 (granny / frying pan) from 23: a quest the template never started (10 entries appended, 4 counters), 11 `mq0004_*` facts, phase scope
  `90836e08` + listeners `54a06fcf`, `b51838dc`, scene `mq0004_old_woman`, spawn set + stub. F1 vs 23: only the two toggling scopes
  (`7456b86a`, `fbda3c84`) differ.
- Quest catalogue of the 44 lukwraith pairs (subagent, read-only): scratchpad `catalog\quest_events.md`; facts used above re-checked by hand.

### 2026-11-05 (later): gates S and F passed; name remap (offline); gate R built

- **Gate S** (user-reported; re-save `C:\w3-fixtures\after_gate_S\S1`): S1 (lukwraith 21 + the user's own spared-Napp branch from donor S)
  loads, quest completed, Napp stands and does not restart the questioning, Willis opens his normal trader dialogue. In the re-save the game
  advanced the transplanted epilogue on its own: `ExternalDialog[mq0005_arsonist]` and thread `4f5f58fc` fired, `7fd9eeae` now waits (the same
  state as lukwraith's real save 22), and the game dropped `c[prologue_smith.w2comm]` (also as in 22). So the game RUNS a transplanted
  frontier, it does not merely tolerate it. Journal unchanged except `JEntryUnread`; no `mq0005_*` fact change.
- **Gate F** (user-reported; re-save `C:\w3-fixtures\after_gate_F\F1`): F1 (lukwraith 22 + mq0004 from 23; the quest never started in the
  template) loads, quest completed, the granny is gone; the frying pan is still in the house but not interactable. The house's investigation
  objects still react (the re-save gained `mq0004_assassin_corpse_found`, `mq0004_breakable_door_*_hit` records with the session's times), but
  the journal is unchanged and the quest did not reopen. **A quest the template never started can be marked done this way** (one quest).
- **Name remap** (subagent; verified here: `cargo test`, clippy clean, fixture tests re-run): `graph::transplant_with(.., NameMode::Remap)`
  translates every NM index of a copied subtree, including inside opaque values whose layouts it decodes (struct `00 {u16 name, u16 type, value}* 0000`
  for `Vector`, `SActionPointId`, `EngineTime`, `GameTime`; `ActivatedInputs` = u32 count + u16 names, all 1,058 elements name `In`..`In 23`;
  suspension data; `BLCK`/`AVAL`/`PORP` streams); anything else is refused. Remap mode replays 86 of 88 pairs byte-exact and 06 -> 07 (names not a
  prefix) by resolved fingerprint; two donors with conflicting names (lukwraith 05 + 06 + 07) compose; whole graphs from 3 other playthroughs
  verify offline. Details: `docs/findings/name-remap.md`. CLI: `journal-set --graph-remap`. In-game untested.
- **Gate R** (built, pending): R1 = lukwraith 21 + mq0004 (donor lukwraith 23, guards re-taken on 21 for the two nodes removed whole) + the
  spared-Napp branch (donor S, the scene list edited point-wise instead of replaced whole, so the granny's scene stays removed). Two deltas from two
  different donors on one template. Remap was not needed here (the granny delta only removes nodes, so no donor names are copied); an in-game
  test of a remapped copy is still open.
- Composition rule found while building R: a delta that REPLACES a shared list (the scene list) undoes other deltas; deltas meant to compose
  must use point ops (remove / insert / replace of the quest's own entries + recount).
- **Gate R passed** (user-reported 2026-11-05; re-save `C:\w3-fixtures\after_gate_R\R1\ManualSave_8559a_7ea51000_520ad2`): R1 loads, both quests
  completed and stay completed, Napp stands, no granny, no reopen. The re-save is `check-save` clean; journal unchanged except `JEntryUnread`
  (objective counters re-hashed by the game); the game advanced the spared epilogue as in S1 (`4f5f58fc` -> `7fd9eeae`, `prologue_smith` set
  dropped); the granny's house objects still record hits (`mq0004_aard_hit_detector_*`) without reopening the quest. **Two deltas from two
  different donors (lukwraith's playthrough and the user's) compose on one template in game.** Remapped copies remain untested in game.

### 2026-11-05 (later): gate M passed: name remap works in game

- **Gate M** (`C:\w3-out\gate_M`; user-reported PASS; re-save `C:\w3-fixtures\after_gate_M\M1\ManualSave_8559a_7ea51000_5e5d17`): M1 has the same
  content as R1 (`graph-diff M1 R1`: 0 changes in `questSystem`, `community`, both journal blocks; facts equal) but is built so that only a remap
  keeps it right: step 1 = 21 + mq0004 from lukwraith 23 plus an identity copy (`questSystem/numQuests`, 10 in both) that makes prefix mode append
  23's 46 extra names (index 3201 = `Brought`, the brought-branch input); step 2 = the spared branch from the user's donor S, whose index 3201 is
  `Released`, applied with `--graph-remap` (prefix mode refuses it). The remap appended `Released` (3247) and `OnGoodDeal`, and the inserted
  `questBlock[4f5f58fc]/inputName` reads `Released`. In game: loads, both quests done, Napp and Willis as in R1, no granny. The re-save shows the
  game advanced the spared epilogue exactly as in S1/R1 (`4f5f58fc` -> `7fd9eeae`, `ExternalDialog[mq0005_arsonist]` fired, `prologue_smith` set
  dropped), kept our 3,248-name table unchanged, `check-save` clean, journal and `mq000*` facts unchanged except `JEntryUnread`.
  **Name remap (`NameMode::Remap`) is proven in game for this case** (CName `inputName` + appended names); other remapped value types
  (struct field headers inside `Vector`/`SActionPointId`, `ActivatedInputs`) were not exercised by M1 and stay offline-proven only.
- Unused appended names are harmless: G1 (36 appended, mostly unused) and M1 (46 + 2) both loaded, and the game re-saves them as they are.

### 2026-11-05 (later): export wired to the questbook

Export now applies `[effects.*.graph]`, journal and fact effects from questbook TOML (details `docs/findings/export-wiring.md`, decisions in
DECISIONS.md "Export of quest effects"). Verified: `W3AL_FIXTURES=C:\w3-extract W3AL_DONORS=C:\w3-fixtures\donor_S W3AL_GATES=C:\w3-out cargo test
-p w3al-core --test export_fixtures`: 6 of 6 pass; G1 and R1 exports have whole decoded payloads identical to the in-game-tested gate saves.
A manual `w3al export --run run.toml --questbook crates/w3al-core/tests/data/gate_r1` (template lukwraith 21, donors lukwraith 23 and donor S)
wrote a file whose payload equals gate R1 (`w3al diff`: 0 regions differ). So the questbook -> export path produces saves already proven in game.

### 2026-11-05 (late): act-2 donors from the Nexus 2394 pack (session V1)

- Pack "Many Save Games" (nexusmods.com/witcher3/mods/2394, file 2-0, downloaded by the user, unpacked to `C:\w3-extract\nexus2394`): ONE
  playthrough, 100 old-gen saves (`[64,18,163]`, `.sav` + `.png`, no `.json`), base game + both DLC, "saved before every important decision"
  (pack page). Research only (third-party). The pack's own line: spirit destroyed, Baron hanged / Anna crazy, Keira to Kaer Morhen.
- The user loaded three "before" saves in 5.01, re-saved them (templates now `[67,29,164]`, check-save clean) and played the branches.
  Branch map: `C:\w3-fixtures\session_V1\BRANCHES.txt`. Captured: Whispering Hillock released / destroyed (`q105_evil_spirit_released` vs
  `q105_betray_spirit` + `q105_evil_spirit_destroyed`, same 8 journal flips); Keira to Kaer Morhen / to Radovid (decision made, `q109wrapup`
  still Active) / killed (`q109wrapup` Success); Letho helped but sent elsewhere (`sq102_helped_letho`, `q402_told_letho_to_go_elsewhere`).
  Letho invited to Kaer Morhen added in session V2 (`ManualSave_52584_7ea51000_a4841f`, `q402_invited_letho_to_km`). Two donors were not played from the re-saved template (names not a prefix): remap needed for them.
- **Design consequence (not yet solved):** a decision's later consequences live on its own line of play: the pack's later saves all carry
  "spirit destroyed" and its Crookback Bog outcome, so a "released" choice needs its own downstream donors (Return to Crookback Bog played on
  from the released donor) before any later template of that line can be used. Content capture is a tree of play from branch points, and the
  exporter must apply decisions onto a template that has not made them yet. To be designed (`docs/plan/`), with an offline study first.

### 2026-11-05 (late): decision chaining study (act 2, offline)

Write-up `docs/plan/decision-chaining.md` (subagent; verified here: 139 tests, clippy clean, `stub_identity_fixtures` 4/4 on
`C:\w3-fixtures` and `C:\w3-extract`: replay 86 pairs, 74 byte-exact, 12 equal up to stub order, 2,242 stub ops by idTag vs 3,508 by position;
idTag unique per save on 201 saves; spot check: 477 stubs in T_letho, no duplicate idTag).
- A decision cannot be switched after it is made (sibling branches differ by journal entries on both sides; journal removal unsupported) [proven].
- Stubs are addressed by `idTag` (`community/stubs/a{idTag=..}`), unique and stable within a line; a respawned NPC gets a new idTag
  (match by spawn set + `entityEntry`); the game drops the universe `entityData` of the same idTag with the stub. CLI: `w3al stubs`,
  `graph-diff --stub-id`, `graph-check --toml D [--base T]`.
- Portability: a delta captured on T_letho does not apply to the earlier T_keira as is (the quest's start triggers stay waiting); a
  "closure" re-captured from T_keira to the Letho donor (34 ops, remap) applies; Keira (KM) + Letho closure compose on T_keira offline. The
  spirit cannot join (its scope ends at the Baron finale); on T_spirit, Keira/Letho would need the whole Velen main path.
- Proposed content model: **decision windows** (cut an act where a decision's consequences end; per window one template per earlier-window
  branch, parked before its decisions; deltas captured from that window template). In-game untested; next gate X1 = T_keira + Keira (KM) +
  Letho (invited) closure.
- **Gate X1 built** (`C:\w3-out\gate_X`, in gamesaves; README in Russian): T_keira (`ManualSave_568ef_7ea51000_923cb5`) + Keira to Kaer Morhen
  (donor `94abd5`: 19 graph ops, journal 1 flip + 4 appends + counter + highlight, facts `q109_keira_to_km`, `q109_confronted_keira`,
  `keira_part1_dd_stop`, then `keira_part1_dd_start` removed) + the Letho-invited closure captured from T_keira (donor `a4841f`, 34 graph ops
  with remap (1 name appended), 33 journal appends (`letho`, `sq102dolores`), 14 counters, 30 `sq102_*`/`q402_invited_letho_to_km` facts).
  Graph blocks byte-identical to the study's verified graph-only build; Letho start triggers `72a761d5`/`15601b24`/`8fdb44ab` absent, waits
  `7573f7fb`/`f5290e77` present; `roundtrip` PASS, `check-save` clean. In-game result pending.
- **Phase 2 exit test, partial (2026-11-05 night):** `cargo tauri build` produced `target\release\bundle\nsis\Another Life_0.1.0_x64-setup.exe`
  (2.30 MiB, exit 0). A no-effect export (run with both sample quests skipped, template the user's 5.01 save) through the same core export path
  the app uses gives a payload identical to the template (`w3al diff`: 0 regions differ). Still to do: a tester installs the app, picks a save,
  answers two questions and exports through the UI.

### 2026-11-05 (night): quest graphs read from the game files (speed-up study)

Study (subagent): scratchpad `speedup\speedup.md`. Game-file extracts and dumps (~286 MB) stay in the session scratchpad and must never enter
the repo (CDPR files); the bundle reader and CR2W dumper used were written from scratch (no WolvenKit code; WolvenKit is GPL-3.0: local viewer only).
- All 1,041 `.w2phase`/`.w2quest` files of the 5.01 install parse; every `questSystem` block GUID of lukwraith 21 (1,023/1,023) and of T_letho
  (2,793/2,793) is a block of those files [proven by the study; spot-checked here: `300f579b` is in `mq0005_dwarven_blacksmith.w2phase`, which
  also holds the `Brought` / `Released` outputs and `mq0005_brought_arsonist`; `38424c39` is in `part_1_sidequests.w2phase`].
- The graphs give block -> quest attribution, fact names per branch, wait types, and journal GUID paths; 98% of frontier blocks added between
  lukwraith pairs are reachable from blocks that fired.
- Ranked speed-ups (study's estimates of user play saved): graph-guided capture planning (~10-20 h), debug console `DBGConsoleOn=true` in
  `bin\config\base\general.ini` (untested on 5.01, ~8-12 h), synthesizing sibling branches from one donor + the graph where branches rejoin
  (~5-10 h; 202 of 994 multi-output decision scenes). First experiment: build "Keira to Radovid" from the Kaer Morhen donor + the graph and
  compare with the real Radovid donor. (KM vs Radovid donors differ in one block of Keira's scope `143c1d62` plus timer/`bob_quest` noise.)

### 2026-11-05 (night, later): branch synthesis works offline; X1 fixed; app prototype

- **Branch synthesis** (subagent; `docs/findings/branch-synthesis.md`, `w3al-core::branch_swap`, `w3al journal-set --swap`): "Keira to Radovid"
  built from T_keira + the Kaer Morhen donor's delta + the quest graph (scene `d24f649d`: socket outputs swap journal entry, fact
  `q109_keira_to_radovid`, wait `f5290e77` -> `c133524a`, character entry), nothing taken from the Radovid donor. Verified here against the real
  Radovid donor `97cdc9`: `journal-diff` empty (order, counters, tracking included); decision facts equal except record times; 0 `graph-diff`
  lines in Keira's scopes (`bb109b9f`, `143c1d62`, `a536c75d`, `ExternalDialog[keira]`). Set as gate K2 (`C:\w3-out\gate_K`, in gamesaves).
  Letho cannot be synthesized this way (choice inside one scene, outputs do not rejoin).
- **X1 flaw found and fixed:** the chaining study's Keira delta (19 ops) left q109's own thread at the pre-decision wait `215f2719` (absent in the
  real donors); rebuilt with `keira_km_q109.toml` (+6 ops): now `215f2719` absent, `d5740aa4`/`f61db075` present, check-save clean. The flawed
  file is in `C:\w3-out\gate_X\old_X1_flawed` and `C:\w3-backup\old_X1_flawed_20261105`; the corrected X1 (same stem) is in gamesaves.
  Lesson: scope selection must include the decision quest's own thread; `graph-check --base` should be run on every delta.
- **App prototype** (subagent; verified: clippy clean, 3 app tests pass on `C:\w3-fixtures`): Welcome (template + donor folders + run.toml),
  quest pages for any questbook, Export (donor status, remap switch default on, Check vs Export, plain-language errors, default destination
  the game's save folder, create_new). Installer `target\release\bundle\nsis\Another Life_0.1.0_x64-setup.exe` (03:45). Not yet run by a person.

### 2026-11-05 (night, end): Velen prototype content, `facts_remove`, helper mod

- **Velen W2 content** (subagent; verified here): `questbook/02_velen/for_the_advancement_of_learning.toml` (Keira: kaer_morhen / radovid /
  killed) and `ghosts_of_the_past.toml` (Letho: invited / elsewhere; closures captured from T_keira, remap). Lint 0 errors; `velen_w2_fixtures`
  4/4 (all 6 combinations export from T_keira through the real export path and verify; `docs/plan/prototype-velen.md`).
- **`facts_remove`** added to the effect schema (questbook.rs, export apply after `facts`, verification with run-order last-wins, lint refuses
  set+remove of one fact; SCHEMA.md). The Keira effects now remove `keira_part1_dd_start` as every Keira donor does; the KM + invited export is
  identical to the rebuilt gate X1 in the NM table, `questSystem`, `community`, both journal blocks and the whole fact table. 148 tests, clippy clean.
- **Debug console / helper mod** (subagent, research only, nothing run in game, no WitcherScript compiler here): `DBGConsoleOn=true` under
  `[General]` in `bin\config\base\general.ini` (the string is in the 5.01 exe); 868 exec functions in the scripts incl. `addfact`/`removefact`,
  `xy`, `tptonode`, `god`, `killall`, `wait`; no console command touches the quest system and scripts cannot enumerate facts. Our original mod
  (`modW3ALDev`: fact read/write, journal dump to the script log, teleport to tag / map pin, NPC list; optional `modW3ALFacts` with known fact
  names from graphs, scripts and saves) and Russian instructions: `C:\w3-out\mod_tools` (not in the repo: the fact-name list is game-derived).

### 2026-11-05 (day): gates X1 and K2 passed (user-reported + our re-read)

Re-saves copied to `C:\w3-fixtures\after_gate_XK`; test files and re-saves moved to `C:\w3-backup\test_saves_removed_20261105_XK`
(hash-verified). The game replaced the X1 file when the user saved over it; the original stays in `C:\w3-out\gate_X`.

- **X1 (Keira to Kaer Morhen + Letho invited, two decisions on T_keira): PASS.** User: Letho's quest done, Keira's quest continues and the
  journal says Geralt sent her to Kaer Morhen; Keira was not at the tower (she has left, as in the played branch). Re-read: journal append-only,
  no entries added or removed; the game itself moved `questact2/q109wrapup` and its objectives `Active -> Success` and switched tracking to
  the main quest; the game set `keira_dd_start` and `achun_CompleteKeiraMetz` (the user's played "KM, done" donor `9abcc7` also has
  `keira_dd_start`). In `questSystem` the pre-decision wait `215f2719` stays absent; `d5740aa4`/`f61db075` were consumed by the game as the
  branch ran to its end. So a transplanted decision does not only display: the game finishes the branch from it.
  Side note: the player spawned in the middle of the river near the tower (template position; not investigated).
- **K2 (Keira to Radovid, synthesized from the KM donor + game quest graph, never played): PASS.** User: journal says Geralt let her go to
  Radovid; the decision did not replay. Re-read: the re-save equals the test in journal (0 differences) and in decision facts
  (`q109_keira_to_radovid` kept; only the 22 time-stamped noise facts changed); `c133524a` still waiting, as in the real Radovid donor right
  after the decision. Not exercised: the branch's later steps (the user saved at once).
- Consequence: branch synthesis (`docs/findings/branch-synthesis.md`) is proven in game for a branch whose outputs rejoin; Velen prototype
  (Keira + Letho) works in game for the combination tested.

### 2026-11-05 (night): donor-free synthesis from the game's quest graphs (`w3graph`)

Write-up `docs/plan/no-human-pipeline.md`. New crate `crates/w3graph` (bundle + CR2W readers written from the files, quest-graph
model, decision walker, save-side frontier reader, synthesis), CLI `w3al synth-walk | synth-apply | graph-lookup`, fixture tests
`crates/w3graph/tests/synth_fixtures.rs` (`W3AL_GRAPHS=<extracted folders;..>` or `W3AL_INSTALL=<game>`, `W3AL_FIXTURES=C:\w3-fixtures`).
- The walker follows a decision output through instant blocks (facts, journal, rewards, scripts incl. `RemoveFactQuest`, spawn
  phases, phase inputs/outputs, evaluable conditions) to the next waits, fires the template's fact listeners the walk made true,
  and `synth::apply` writes the result (frontier blocks and scope wrappers built from the save layouts, scene registry, journal
  with Success propagation, facts) into a copy of the template. No donor is read.
- Checked against the session V1 donors [proven offline]: Keira KM and Radovid: prediction 9/0/0 blocks (hit/miss/extra), 4/0/0
  facts, 5/0/0 journal; the synthesized saves have 0 `questSystem` differences in the quest's four scopes, 0 journal differences,
  equal decision facts. Spirit released: 34/0/0 blocks, 14/0/0 journal; destroyed: 12/0/0, 13/0/0; journals of the synthesized
  saves equal the donors'. Fact misses are scene-file facts (`q105_betray_spirit`), script effects (`q105_new_heart_killed`) and play
  noise. Not written yet: `community` (spawn phases/stubs), objective counters, scene-file facts.
- Reading the real install works (`graph-lookup --install`, 3.9 s, read-only). Option 2 (the game as an unattended runner): script
  APIs catalogued, draft mod + README in `C:\w3-out\auto_runner` (not compiled, not launched).
- Next: gate K3 (synthesized Keira KM with community from the graph, next to K2), then scene facts, counters, questbook `synth` effects.

### 2026-11-05 (night): synth content for W1, W2 rest, W3, W5, W6 (questbook) and walker issues found

Files: `questbook/02_velen/{whispering_hillock,a_favor_for_a_friend,ghosts_of_the_past}.toml`,
`questbook/03_novigrad/{get_junior,now_or_never,the_last_wish}.toml`, `questbook/05_kaer_morhen/brothers_in_arms.toml`; each
names its template and its `synth-walk` arguments. Donor-checked: D1 both branches (34/0/0, 12/0/0 as before), D3 night (pack 12 ->
13, old-gen, 17/0/10), D6 kill (pack 21 5.01 -> pack 22 old-gen, 72/4/12); D8 stays and D9 friends checked only for their decision
facts (pack 43 is a whole act later). Graph-only: D3 dinner / no date, D4 abandoned, D6 spared, D8 leaves, D9 love.
Open questions:
- **Walker issue W-1 (`w3graph::walk`)**: activations queued inside a phase keep running after the phase has exited through
  another output. Effects: a `--through` walk leaves through every output of a multi-exit phase (`101a1f03` leaving estate: front,
  back and sewer gate, re-entering "To HMS" three times; Ambush `608342b6` ran `Out` and then `Fail`, setting `q309_failed`), and a
  `stop` meant to suppress the other exit parks a wait inside the already ended scope (extras in every comparison above). The
  questbook works around it with stops; the fix belongs in the walker (drop queued work of ended scopes).
- **Walker issue W-2**: `CJournalQuestBlock` input `Failure` is mapped to `JS_Active` (walk.rs only knows `Fail` / `Failed`).
  Affects D3 no date (sq101 root), D4 abandoned (sq102 root, `a66421b0`), D6 spared (`ff269d2c`), q309 fail paths.
- **No date with Keira (D3)**: sq101's `Out_no_date` is not wired in `part_1_sidequests.w2phase` (only `Out` -> q109 start `3673c07f`).
  Does q109_wrapup (D2) start another way after a refused date? Needs a played save or a wider graph search.
- **D12 template**: pack 43 is still inside q401 (Ugly Baby); the q402 "gathering allies" phase `b74543d1` is not entered, and a walk
  through q401 meets ten unresolved points (including the q401 Yennefer scene `2e706306`, a romance branch of its own). Needs a 5.01
  save inside `b74543d1` with no ally asked; until then the D12 effects are placeholders. Hjalmar's deciding dialog block is not traced.
- Pack 12 (D3 template) has no 5.01 re-save yet; its walk arguments were read on the old-gen save.

### 2026-11-05 (night): synth content for W4, W7, W8, W9 (questbook) and walker issue W-3

Files: `questbook/04_skellige/{lord_of_undvik,kings_gambit}.toml`, `questbook/06_act3/{blood_on_the_battlefield,final_preparations,
reason_of_state,child_of_the_elder_blood}.toml`; each names its 5.01 pack template (`C:\w3-fixtures\pack_501`) and its walk
arguments. Every walk was run with `w3al synth-walk` on the 5.01 re-saves; chained windows (W7, W9) were checked by writing each
step with `synth-apply` and walking the next step from that file. Donor-checked (old-gen pack "after" saves): D10 Folan lives
(pack 34), D13 snowball + D14 Imlerith (pack 49), D16 faced alone (pack 52), D18 Dijkstra side from the pack 52 bridge (pack 53),
D19 lab wrecked + D20 grave visited (pack 55). Graph-only: D10 Folan dies, D11 refuse / council, D13 talk, D14 Emhyr, D15 both,
D16 help, D17 both, D18 from the D17 chain and Roche side, D19 necklace, D20 skipped.
Findings:
- **D18 socket -> fact (the catalogue's open point)**: `mq3035_wrap_up.w2phase` scene `de41f700` `Fight_output` = side with Roche
  (fight -> `65fb9638` `mq3035_fdb_dijkstra_dead`, `c297b437` `mq3035_free_temeria`); `Leave_output` = side with Dijkstra (`0026e3bd`
  `mq3035_fdb_roche_talar_dead`). Confirmed on pack 52 -> 53 (Leave_output, Dijkstra boards).
- **D17** has graph sockets (`51cf714f` `mq3035_agrees_to_plan` / `mq3035_disagrees_to_plan`), not scene facts; refusing ends the
  meeting through an unconnected output and mq3035 fails at `09b34372` [q501_naglfar_arrived]. Pack 52 is already inside the
  Radovid fight, so pack 51 (after D16) is the real "before" save of D17.
- **D16 template**: pack 51 is before Philippa's rescue; the D16 walk runs the rescue with the pack line's answers, including a
  choice the catalogue does not list (Dijkstra's leg, `fc64e6ac` `out convinced` / `out broken leg`, `q310_dijkstra_spared`).
- **D20**: decided at `4624122f` (`q210_13k_outside_lab_skjall`), not inside the grave phase: `Output_notime_skjall` leaves through
  an unconnected phase output and the grave phase never runs.
- **D10**: only Folan is a dialogue choice (`98ac08c2`). Vigi's fate follows `q202_geralt_went_for_giant` (`3f497c4f`, set when the
  player wakes the giant before freeing the cage; condition `e94bf2dc` picks cage scene `e485db7a` = Vigi dead or `4cdfd32c` = saved),
  i.e. play, not a choice. Open: offer it as a synth branch (pick the lair's phase outputs) or leave it to play, as now.
- **D11 Cerys / Hjalmar**: through-walks of the two investigations do not finish (Cerys: barrel scene `5884be0a` and an ambiguous
  importer of `random_timer.w2phase`; Hjalmar: the path scope is lost, likely W-1). Offered as "the save stops at the council,
  decide in game" until the walks are fixed.
- **Scene-file facts set by effect `facts`** (names read by graph blocks): D15 `q110_refused_reward` / `q110_accepted_reward` +
  `q110_geralt_refused_pay` / `q110_geralt_accepted_pay`; D19 `q210_gave_lara_necklace` / `q210_cs_ciri_destroy_lab`; D20
  `q210_skjall_done`. Listeners waiting on those facts are either started explicitly (D15: `26b11e5d` / `1af99c14`) or left for the
  game to fire on load (`1f3226ea`); walking the necklace listener loops the "Ciri Necklaces" phase.
- **5.01-only listeners**: the NGE `bugfixing_phase.w2phase` adds `8752d51c` [q110_completed > 0] -> journal-status condition
  `1d2f15fd`, which the walker cannot evaluate; old-gen templates do not have it. Stopped (left for the game) in D14/D15.
- **Walker issue W-3 (`w3graph::walk` / `synth`)**: a `CQuestCutControlBlock` reached inside a phase stays in the predicted frontier
  after that phase (and its parent) has exited; `synth-apply` then fails with "questThread is not in the save". Blocks export of
  D16 (both, `98d0449f` in Philippa's phase `ef9ca5e4`/`6f4bbbf8`) and D11 refuse (`95d1d2e0` in the ended q206 scope `6cc212b3`);
  D19 writes but keeps `6d9a2260` in the ended `6842c7e0`. Because D17/D18 start from D16's result, W8 cannot be exported until
  W-3 is fixed. Related to W-1 (queued work of ended scopes).
- **Question flow**: the schema has quest-level `requires`/`excludes` only. D15 (needs D14 = Emhyr) and D18 (needs D17 = agree)
  are second choices of the same quest with a "none / decide in game" option; a contradictory answer makes export refuse (the
  start block is not waiting). A per-option condition (`when = "ride=emhyr"`) would let the app hide the page.

### 2026-11-05 (late): synthesis gaps closed (community, scene facts, counters), walker fixes W-1/W-2/W-3, every synth effect exports

Code: `crates/w3graph` (`community.rs`, `scene.rs`, `resources.rs`, `walk.rs`, `synth.rs`, `frontier.rs`), `w3al synth-apply --donor`
prints the community comparison, `w3al scene-info`, `w3al graph-lookup` lists a block's input sockets and feeders. Tests:
`crates/w3graph/tests/synth_fixtures.rs` (5 tests, `W3AL_INSTALL` + `W3AL_FIXTURES`), unit tests per walker rule.
- **Closed vs the played donors** (numbers in `docs/plan/no-human-pipeline.md` section 2): spawn-set phase switches, new `c`
  blocks from the `.w2comm` tables, deactivated sets with their stubs (swap-remove by idTag) and `entityData`; the facts set inside
  `.w2scene` files on the way to the output taken (`q105_betray_spirit`, Letho's `q402_invited_letho_to_km` with
  `--scene-pick "ed477b93=sq102_go _to_km_letho"`); `count 0` counters for new `CJournalQuestObjective` entries. Keira KM /
  Radovid, spirit released / destroyed: journal and counters 0 differences, facts 0 extra (misses = script side effects and
  play), quest scopes equal, community equal up to the stubs the game creates for a newly active phase/set [hypothesis, gate
  K3]. Letho invited (session V2): facts 0 miss / 0 extra, community 0 differences, the quest's And block byte-equal.
- **Walker**: W-1 (queued activations of an ended phase dropped; the note names them), W-2 (`Failure` -> `JS_Failed`), W-3 (a
  sub-phase entered inside a phase that ends afterwards leaves with it; D16 lodge and D11 refuse write). New: `CQuestAndBlock`
  waits until every connected input is activated, seeded with the template's saved inputs (`inputNamesCount` > 1, `ActivatedInputs`:
  pack 51 `a73fe883` held `In 2`, `In 1`; Letho's `e9e84a64` waits with `In 1` as the donor does); a fresh And wait writes one
  `inputName` per input plus `ActivatedInputs`. A re-entered phase's old inner wrappers are not reused as paths (q309 Restart loop,
  now_or_never). `CActivateStoryPhase` with the unnamed default phase writes CName 0.
- **Export**: `[effects.*.synth]` takes `scene_pick`; export's touched blocks are exactly what `synth::apply` writes (`questSystem`,
  journal, `facts`, `community`, `universe`). Every questbook synth effect on a 5.01 template exports VERIFIED (export's own checks:
  predicted frontier, journal, facts, untouched blocks byte-identical, check-save): whispering_hillock released / destroyed,
  ghosts_of_the_past abandoned, get_junior killed / spared, now_or_never stays / leaves, kings_gambit council / refuse,
  lord_of_undvik lives / dies, blood_on_the_battlefield snowball|talk + imlerith|emhyr + refuse|accept (chained, run order),
  final_preparations alone|help + reason_of_state agree + dijkstra|roche and refuse (chained), child_of_the_elder_blood
  necklace|wrecked + visit|skip (chained). a_favor_for_a_friend skipped (no 5.01 template).
- **Content fix needed (questbook, not changed here)**: `the_last_wish` love / friends are an in-scene choice
  (`sq202_10_ending_djinn.w2scene`): the scene reader now surfaces `sq202_despawn_yen_end` behind it and export refuses both until
  `[effects.love.synth]` gets `scene_pick = ["b0225ae2=sq202_10_lets_stay_together"]` and `[effects.friends.synth]`
  `scene_pick = ["b0225ae2=sq202_10_forever_alone"]` (verified from a scratch copy: both export, `sq202_yen_girlfriend` /
  `sq202_yen_just_friends` set). Letho `invited` / `elsewhere` could now be synth effects on T_letho (the walk above; elsewhere =
  `ed477b93=sq102_find_shelter`) once the per-window template exists. A run that exports an act-3 main quest must mark the
  earlier main quests `started` (resolve locks a main quest after a skipped one).
- Open: the `through` walk passes fact waits the donor's player had not reached (`5f88ff5b` in Letho's "Fake-ass dead guys" phase);
  a `stop` on them matches the donor exactly. `sq102_hide_community` ends at 2 (donor 0: a later script zeroes it);
  `sq102_explode_building` is set with an expiry the walk does not model.

### 2026-11-05 (evening): all questbook decisions export from the game files; gate Y built

- Orchestrator re-check of the Fable synthesis pass (details in the entry above and `docs/plan/no-human-pipeline.md`): applied its
  content fix (`scene_pick` for Yennefer love / friends in `questbook/03_novigrad/the_last_wish.toml`); from the repo questbook
  `w3al export` gives VERIFIED for Yennefer love and friends (`sq202_yen_girlfriend` / `sq202_yen_just_friends`), the act-3 chain
  lodge help + plot + Roche (`mq3035_fdb_radovid_dead`, `mq3035_fdb_dijkstra_dead`, `mq3035_free_temeria`) and Skellige act 3
  necklace + grave. Lint 17 quests, 0 errors; clippy 0 warnings; `cargo test` green except the known
  `facts_round_trip_byte_exact_on_every_fixture` when `W3AL_FIXTURES` is the whole `C:\w3-fixtures` (old-gen root saves with lone
  surrogates; pre-existing). Two tests updated for the new content (`questbook.rs` order check, `w2_quests_are_wired` extra synth option).
- **Gate Y** (`C:\w3-out\gate_Y`, in gamesaves, labelled TEST Y1..Y5): first in-game test of saves built with no donor at all:
  Y1 spirit released (T_spirit), Y2 Triss stays + Yennefer love together (pack 42), Y3 snowball + Emhyr + reward accepted (pack 48),
  Y4 lodge help + plot + Roche side (pack 51), Y5 necklace + Skjall's grave (pack 54). Result pending.
- **Gate Y result (user-reported + our re-read, 2026-11-05 evening): Y1, Y2, Y3 PASS** ("all works"); Y4, Y5 not played yet (still in
  gamesaves). Re-saves in `C:\w3-fixtures\after_gate_Y`; test files moved to `C:\w3-backup\test_saves_removed_20261105_Y123`.
  Y1 (spirit released): journal append-only, only unread flags changed; `q105_evil_spirit_released`/`_done` kept. Y2 (Triss + Yennefer
  together): decision facts kept (`q309_triss_lover`, `sq202_yen_girlfriend`), no fact added or removed by the game; **the game itself
  removed one journal entry** (an innkeeper Gwent objective, `cginnkeepers/833d33e1/532a739f`, Active) — first sign that the engine can
  drop journal entries, relevant to the "decisions cannot be switched" limit. Y3 (snowball + Emhyr + reward accepted): `q110_accepted_reward`
  / `q110_geralt_accepted_pay` kept, only unread flags and `q403_horse_fixed` changed. Conclusion: saves built with no donor, from the
  game's quest graphs only, load and keep their decisions in game.
- Y4 (act-3 chain lodge + plot + Roche) and Y5 (necklace + grave) were not played: the user accepted them on the Y1-Y3 result plus the
  offline export verification; moved out of gamesaves to `C:\w3-backup\test_saves_removed_20261105_Y45` (originals in `C:\w3-out\gate_Y`).

### 2026-11-05 (night, later): W6 template synthesized from pack 43; the four ally invitations are real (no placeholders)

Problem: decision window W6 (Brothers in Arms, D12 x4) needs a 5.01 save inside q402_ciri's "gathering allies" phase `b74543d1`
with no ally asked. A scan of every save in hand (298 files: `C:\w3-extract\*`, `C:\w3-fixtures\*`, `C:\w3-backup\user_saves_stash`;
`w3al tree --grep "questBlock["` for the q401 / q402 scopes plus `w3al facts`) found none: the only saves with the q402 scope are
pack 44 (old-gen `ManualSave_6b4a7_7e10d400_55c9e52`) and the two `6b4a7_7e12cc00_4e232cb` fixtures, all with the four invited.
So the template is synthesized from pack 43 (5.01 re-save `ManualSave_100840_7ea51000_3f9b30a`, early in q401):
- **Walk** (`questbook/05_kaer_morhen/ugly_baby.toml`, `done_effects`): start `6e852741` [q401_everybody_returned > 2] (Pre-Reunion;
  the three parallel searches are not walked: a `--through` walk of the forktail hunt / lake trip takes wrong parallel exits, e.g.
  Lambert leaving, and the game drops their scopes with q401 anyway), `--through`, 11 picks following the pack line (Yennefer night
  `2e706306=Output_no_love` since the line has `sq202_yen_just_friends`, drunk evening, Ida call with the dress-up, Eskel fetches the
  potion, nobody cooks, three journal-status conditions), 15 stops = the ally context dialogues of `b74543d1`, the two
  `q402_cerys_sent_hjalmar` waits, `sq312_ves_died`, and `41d211a6` of "conditions for reaching island of mists". 810 steps: q401 and
  Part 1 (`2ea0d885`, `a0881c9f`) end, `e526fd7b` / `9b9a1ec2` / `b74543d1` start with every ally dialogue waiting (Mousesack `4075a745`,
  Crach `8f663595`, Dijkstra `cfa72dbe`, Roche `4a92523c`, Zoltan `22f5025a`, Emhyr `2cac1202`, Cerys `0cd4179d`, Hjalmar behind
  `488dee9d`), the And blocks `377b6ab2` (Keira's `In 4`) and `73ef275d` (Triss's `In 3`) as the game leaves them.
- **Against pack 44** (`synth-apply --donor`): every q401 / Part 1 frontier block pack 44 lost is removed (no miss in those scopes); the
  q402 differences are exactly the allies (pack 44 invited all four and talked to Crach, Emhyr and Dijkstra); facts: q401_completed,
  chapter1_completed, q402_know_about_battle hit; misses = play of the skipped searches (q401_hunted_forktail_down,
  q401_charged_phylactery, gwent, ...; their only readers after q401 are character-journal listeners the game fires on load) and
  pack 44's Crach / Emhyr (q402_talked_to_emhyr, q402_hide_sword, q402_crach_will_teleport). Journal: q401's root Success flips its
  entries as in pack 44; the two search sub-quests (q401_hunt_with_eskel, q401_phylactery) are closed with the tables captured by
  `w3al journal-diff` pack 43 -> 44 (in the same effect; `at` dropped).
- **Allies** (`questbook/05_kaer_morhen/brothers_in_arms.toml`): Ermion = `4075a745=q402_ask_mousesack_for_help_out` (one output);
  Zoltan = `22f5025a=q402_zoltan_km_ally_out`; Roche = `4a92523c=q402_ask_roche_for_help_out` + journal-status conditions
  `07c9c338` / `802f7cb6` False (both entries already set), scene fact `q402_roche_helps_context_dialogue`; **Hjalmar traced**: on the
  pack line (Cerys queen, q208 done) the live dialogue is `c9e4e916` in "Context to default" `26a1b620`, gated by `488dee9d`
  [q402_cerys_sent_hjalmar], which the Cerys dialogue `0cd4179d` sets through `q402_cerys_sends_hjalmar` (`89b38833`); the Cerys
  phase closes with `01e45e4e q402_cerys_refused_km` either way; `c9e4e916=q402_ask_hjalmar_for_help_out` -> `7ac368c6` ->
  `0d9600fd` -> `2b758a00 q402_invited_hjalmar_to_km`, scene fact `q402_hjalmar_invited_to_q403`. One effect: start `0cd4179d`, two
  picks; the listener-fired Hjalmar dialogue honours the pick. "Not invited" = no effect (the dialogue keeps waiting for play).
  Crach, Emhyr, Dijkstra, Keira, Triss are not decided here (Keira's and Triss's phases close by facts; the others wait for play).
- **Exports** (`w3al export`, run files in the session scratchpad `allies\runs`): the template alone, all four, none, Ermion + Roche,
  Hjalmar + Zoltan: all **VERIFIED**. The exported template equals the `synth-apply` file in `questSystem` and `community` (byte-identical)
  and facts; all four vs pack 44: the eight decision facts equal (`q402_invited_{mousesack,hjalmar,roche,zoltan}_to_km`,
  `q402_allies_gathered`, `q402_cerys_sent_hjalmar`, `q402_cerys_refused_km`, `q402_hjalmar_invited_to_q403`), one extra
  (`q402_roche_helps_context_dialogue`, 5.01 scene file), pack 44's five extra q402 facts are Crach / Emhyr / the regional And blocks;
  q402 journal: the roots and phases exist (see below), 17 status / unread differences all from Crach, Emhyr, Dijkstra and reading,
  5 entries pack 44 added by later play.
- **Walker rules learnt** (`crates/w3graph/src/walk.rs`, unit tests each): **W-4** a file imported by two phase blocks in a row
  (`weapons_unmount.w2phase` by `719f4f02` then `09cd76c3`) is walked once per importer (the `visited` dedup was global; the second
  Unmount Weapons did nothing and the Reunion never ran); an importer the walk has left is no longer "running" for the parent lookup.
  **W-5** a wait assumed satisfied yields to the instant blocks queued with it: what follows the wait goes to the bottom of the work
  stack (Roche's `q402_ask_roche_for_help_out` activates the despawn chain and journal `6fb13bcc` at once; the chain reached the phase
  exit after a 1 s pause and the journal block was dropped). With it, starts fire one after the other, each walked to the end
  before the next, and a start whose phase an earlier start ended does not fire (spirit donor: `c2549952` ends "leading horse" and
  `c4d54c91` never fires; both orders are unit-tested). **W-6** a phase re-entered through another input socket runs its content
  again (Cerys `ed29bf82`: `First`, out through `To_q208`, back through `Second`; the dialogue inside waits once more); the same socket
  twice (a Restart loop) keeps the old visits, so a loop makes one lap per socket.
- **Journal rule learnt** (`synth.rs`): activating an objective of a quest the log does not know creates the quest's root and phase
  entries (every path prefix of depth >= 2) as Active: pack 44 has `questact2/q402ciri`, `q402ciri/dd17fe27`, `q402skelligeallies`,
  ... although only their objectives have journal blocks; a root set to Success flips them with the rest. [HYPOTHESIS from pack 44,
  not yet seen in a 5.01 re-save of a synthesized quest start.]
- Open: `q402_roche_helps_context_dialogue` is set by the 5.01 Roche scene on the way to the invite; pack 44 (old-gen) lacks it. The
  walk-order change (W-5) is a model of the game's tick; the five fixture donors still match. In-game gate for W6 (load the all-four
  export, check the council scene / q403 allies) not run yet.

## 2026-11-05 content C: more base-game decisions (walker findings and open questions)

Written: Payback + derived Tango (`final_preparations.toml`), Skellige ruler (Hjalmar, Svanrige), Vigi dead, Carnal Sins, A Towerful of Mice.
Facts and blocks come from the graphs and from donor checks (pack 30 for Carnal Sins, pack 12 for the Graham kiss); the third-party
editor data was used only as a lead.

Walker / writer findings (exact arguments in the quest files):
* Cerys queen (pack 35): the walk reaches `q206_cerys_queen` but keeps one unresolved point `fcf456e4 PhaseOutputBlock (random_timer.w2phase)`,
  "2 running importers"; `--pick fcf456e4=Out` is refused ("has outputs []"). Needs importer disambiguation in the walker, or a 5.01 re-save of pack 36 as donor.
* Hjalmar king without a stop at q208: the writer refuses "scope edb2abb3 is not in the template and has no parent there"; the walker followed the
  importer `edb2abb3` of `quests/demo_files/hands_on_q206_berserkers` into q208_heroesmead. Workaround: stops `9afd96a3`, `1bf88c9e`.
* Vigi saved: the through-walk takes the "giant woken first" branch (every wait counts as done); a race cannot be expressed. Donor: pack 34 5.01 re-save.
* Ves (An Eye for an Eye, template pack 21 5.01): the template waits before acceptance (`88ca136b` accept_roche/refuse_roche); after the accept walk
  the kill phase `8e85541b` is entered but leaves no waits in the frontier, and Ves's fate is combat (`sq312_kill.w2phase`), so no walk yet.
* Botchling: pack 6 is before the Family Matters investigation and pack 7 after the ritual; a greedy auto-pick walk from pack 6 reaches 24 picks and
  stops at `89a71b6b` (PhaseOutputBlock of q102_portcullis, "has outputs []") without reaching the ritual (`Botch Spot` `6d539322` -> `Peregrination`
  `fa69d6ac` -> Bury `43a06a4c` q103_buried_botch / Fight `1f51fdb1` q103_killed_botch). A mid-quest save before the ritual is needed. Placeholder stays.
* Lambert (`q403_first_square`, scene `12199345` keira_saved_lambert / lambert_in_danger): needs a template inside the battle (pack 47 old-gen is the
  council end); not walked. Sara, Greedy God, Woods, Wolf's Clothing, Dangerous Game, contracts: not reached.

Re-saves wanted (old-gen pack saves, `C:\w3-extract\nexus2394\gamesaves`, load in 5.01 and save at once): pack 29 `ManualSave_68fb9_7e105800_5df4ef9`
(Carnal Sins), pack 11 `ManualSave_4dcec_7e104000_538e03f` (Towerful of Mice), pack 36 (Cerys donor), pack 34 `ManualSave_4dcea_7e106000_5537644` (Vigi saved donor),
pack 47 `ManualSave_4cb7b_7e10e800_4b1593c` (Lambert, later), pack 17 `ManualSave_4dceb_7e104800_5b01d8c` (Sara, later).
Open: `questbook/templates.toml` has no place yet for `carnal_sins` and `a_towerful_of_mice` (lint warns).

### 2026-11-05 (night, late): story chain: automatic template choice, earlier decisions rewritten on a later template

Design and tables: `docs/plan/story-chain.md`. Code: `crates/w3al-core/src/{templates,chain}.rs`, `w3save::journal::remove`,
`w3al chain-plan | template-scan`, `w3al export` without `--template`, app command `template_plan`. Registry
`questbook/templates.toml` (10 window templates, line = pack number).
- **Encodes derived** (`w3al template-scan`, every answer combination of each window exported on its own template, matched
  by varying facts): the pack line = spirit destroyed, Keira KM, Letho invited, Junior killed, Folan lives, Triss stays,
  Yennefer friends, all four allies, snowball + Imlerith, Dijkstra spared + lodge alone, plot + Dijkstra side; 0
  mismatches for W1-W4, 1-4 transient facts for W5-W8. Not expressible: King's Gambit (pack plays Cerys: `in_game`), Vigi
  (`play`), Keira's date (no 5.01 pack 12; `sq101_keira_had_sex` on every later template).
- **Journal removal is what the game does**: the gate Y2 re-saves dropped `cginnkeepers/833d33e1/532a739f`, later also
  `sq315talar/930b99e5/7441fc8e`, `sq202yen/ef808288/9a3dcf80`, `maincharacters/yennefer/2486878c` and one counter; the
  other entries kept their order, `Size` went down. `journal::remove` does exactly that (unit test: removal undoes an
  append byte for byte).
- **Validation on played pairs** (`story_chain_fixtures.rs`, references synthesized without donors): spirit destroyed <->
  released (5 decision facts, 3 journal entries, 24 waits), Keira KM <-> Radovid (2 / 4 / 2), Letho invited <-> elsewhere
  (2 / 3 / 2): after the rewrite **0 differences** from the other played save in every case, both directions.
- **Full story on pack 54** (Triss let go, Yennefer love, talk, Emhyr + pay, lodge help): template chosen automatically,
  W5 / W7 / W8 rewritten, W9 walked, VERIFIED, check-save clean. Pack 43 with Keira to Radovid, Letho elsewhere, Junior
  spared, Folan dies, Triss let go, Yennefer love: VERIFIED (Keira's KM listener `f5290e77` swapped for `c133524a`).
- Cross-checked against a third-party editor's data (not copied): its per-option fact sets for the spirit, Keira, Triss,
  Yennefer, the Emhyr visit, the snowball and the lodge are each contained in the facts our references differ by (ours add
  the walk's other facts, e.g. `q309_asked_triss_to_stay`, `q110_accepted_reward`); no disagreement.
- Open: no in-game load yet (gate: the two exports above); scene-file readers are not part of the reader scan; the
  hub-file list is by name.
- **Walker fixes (content agent's report, `crates/w3graph`)**: a phase output in a file imported by several running phases
  now exits the importer the walk itself entered (`random_timer.w2phase` `fcf456e4` on pack 35); graphs under
  `quests/demo_files/` are no longer importers (`hands_on_q206_berserkers` led the walk into q208). Unit test
  `a_phase_output_exits_the_importer_this_walk_entered_and_demo_files_never_import`. The King's Gambit Cerys path on pack 35
  (`--start 3daf7b27 --through --pick 31f4501a=Cerys --pick 5884be0a=TheBarrel --pick ba006c80=True --pick 127635cd=Now`)
  now walks with no unresolved point (1,626 steps); not exported or compared with a donor yet. Registry: `carnal_sins`
  (pack 29) and `a_towerful_of_mice` (pack 11) placed on the line (no 5.01 templates yet); `kings_gambit.side` (any
  change, incl. the new `hjalmar`) fixed from pack 42 on; `lord_of_undvik.vigi` recorded as `play`.

### 2026-11-05 (late evening): gate Z built (story chain + start kit), installer rebuilt

- Installer rebuilt (`target/release/bundle/nsis/Another Life_0.1.0_x64-setup.exe`, 21:42): redesigned UI, ru/en UI with official
  game names read from the player's install, template plan, start kit. Full `cargo test --release` with `W3AL_FIXTURES` green except the
  known `facts_round_trip_byte_exact_on_every_fixture`; clippy 0; lint 20 quests, 0 errors.
- Gate Z (`C:\w3-out\gate_Z`, in gamesaves as TEST Z1/Z2): Z1 = pack 54 with W5/W7/W8 rewritten + start kit (level 86 -> 30, 5000 crowns,
  Gryphon gear); Z2 = pack 43 with Keira Radovid, Letho elsewhere, Junior spared, Folan dies, Triss gone, Yennefer love rewritten, four allies,
  start kit level 25. Both check-save clean, decision facts as chosen. Also in gamesaves for re-save: packs 11, 29, 47, 17. Result pending.
- **Gate Z result (user-reported + our re-read, 2026-11-05 late evening): Z1 PASS, Z2 PASS.** Re-saves in
  `C:\w3-fixtures\after_gate_Z` (Z1 -> `ManualSave_4dcef_7ea51000_5792d12`, Z2 -> `ManualSave_101ca7_7ea51000_57b19b5`); test files moved to
  `C:\w3-backup\test_saves_removed_20261105_Z`. In the game's own re-saves: level 30 / 25, XP, `Lvl` abilities and skill points as written;
  crowns 5000 / 3000; the kit items (Gryphon swords, armour, boots, Swallow 1, Cat 1) present with durability 100; journal identical to
  the test file (0 entries changed); only world-noise facts changed. So rewriting earlier decisions on a later template, level edits
  (including lowering an NG+ level 86 character) and new item records are accepted by the game. Packs 11, 29, 47, 17 were not re-saved yet.

### 2026-11-05 (night, side quests): auto-complete walk, generated side-quest catalogue, Side quests page

Code: `crates/w3graph/src/{complete,catalog}.rs` (walker hooks in `walk.rs`: `Policy::auto`, `Scores`, `AutoPick`; suspended scopes in
`frontier.rs` / `synth.rs`), `crates/w3al-core/src/side.rs` (`questbook/side/` loader, lint, `run.side_quests`, classification),
`[effects.*.synth] complete = "GROUP/QUEST"` (`synth_effect.rs`), CLI `w3al quest-info | side-complete | side-check | gen-side-quests |
res-dump | node-dump`, app command `side_quest_states`, `game_names.side`, `ui/side.js`. Schema: `questbook/SCHEMA.md` "Auto-complete mode"
and "Side quests". Tests: `complete.rs` unit tests (policy), `crates/w3al-core/tests/side_fixtures.rs` (pack line + install).
- **Policy** (deterministic, every pick listed): starts = the quest's waits in the template that lead to its journal root's Success,
  furthest first; inside the quest a wait counts as done when an output leads to the goal, the nearest output is taken where there are
  several, a wait with no route (failure listeners, despawns) stays waiting; unevaluable conditions / random blocks take the nearest
  output, else the first; fact blocks lead to the pause / condition blocks of the quest that read the fact (parallel branches meet through
  facts: mh206's investigation sets `mh206_lair_found`, the combat waits for it); the quest's phase output is held back until every start
  was walked; goal = root Success, then the phase's end; edges into the root's Failure never taken. Outside the quest no `through`; the
  save's fact listeners fire, one whose run meets an undecidable point is rolled back and left for the game.
- **Quest model**: `CJournalQuest` (`guid`, `parentGuid` = group = first journal step, `type` as a name-table enum, `world`, `title`
  string id, optional `questPhase`). Home graph = `questPhase`, else the topmost file holding the quest's journal blocks (world hubs
  aside), else the deepest embedded phase holding them all; instances = importers running in the template. Most side quests' graphs run
  in `part_1_sidequests.w2phase` (`117e4006`) before the journal knows them (root absent, waits for the area trigger).
- **New format finding [HYPOTHESIS: offline only]**: while the player is in another world (Kaer Morhen pack 43/48, pack 54, pack 35)
  the hub's quest phases are not threads but entries of the running thread's `suspendedScopesData`:
  `u32 count, n x { 00, u16 name, u16 type, <16-byte raw node: the phase block GUID>, u16 name, u16 type, <raw node: a BLCK/AVAL
  stream with the thread state>, 00 00 }` (pack 43: 35 entries under `117e4006`). Read and re-encoded byte for byte
  (`frontier::suspended_entries`); a suspended quest is walked from its entry and its entry is dropped (facts the save already has are
  not added again). Whether the game accepts a dropped suspended entry is the first thing the in-game gate must show.
- **Validation** (`w3al side-check A B` over the 8 consecutive 5.01 pack pairs; quests B finished and A had not): 37 completed, 90
  refused (40 no quest graph: journals written by scripts, scavenger-hunt notes, points of interest, boat races; 17 decision quests;
  15 no route to Success; 14 not running in A; 4 unresolved). Of the 37: root Success in both 37/37, phase ended the same 31/37, journal
  entries of the quest 385 hit / 99 miss, the quest's facts 184 hit / 111 miss (misses: play facts of alternative routes, kill counters,
  dialogue lines behind choices the policy did not take). Fixture sample of 16 (gwent, contracts, side, main, scavenger; table in
  `side_fixtures.rs` output): journal 195/20, facts 92/21, phase ended in agreement 16/16.
- **Generator** (`w3al gen-side-quests --install ... --out questbook/side --template-dir C:\w3-fixtures\pack_501 --template-dir
  C:\w3-fixtures\session_V1`, 5.5 min): 416 install quests; skipped 109 expansion (HoS / BaW, out of v1), 15 demo/test, 22 whose graph
  the decision questbook walks; of 270 candidates **99 written** (98 side: 65 side, 19 contracts, 6 Gwent, 3 treasure, 2 scavenger, 2
  races, 1 fist fight; 1 main), refused: 90 no quest graph, 36 not running on any template, 17 no route to Success, 11 decision quests,
  11 done on every template, 5 unresolved, 1 phase still running. Per template ok / done: w1 59/11, w2 55/15, w3 54/21, w4 68/41,
  w4b 63/45, w5 51/75, w6 (pack 43) 51/78, w7 45/86, w8 47/87, w9 (pack 54) 42/87.
- **Exports**: 30 generated side quests on pack 43 and 30 on pack 54 (`run.side_quests`, manual template) both **VERIFIED**, check-save
  clean (scratch `side\out`); the fixture test repeats both. Not loaded in game yet.
- Open: graphless quests (scavenger notes, POI treasures, boat races) need a journal-only mode or the entity state; quests whose phase is
  not running and not suspended need their hub gate walked (only same-graph gates are found); in-game gate for a dropped suspended entry
  and for a contract finished while its world is loaded.
