# Branch synthesis: a decision branch built without a donor that played it

Date 2026-11-05 (night). Offline experiment, no game. Labels: **[proven]** = shown on real saves by a command or test named
here, **[inferred]** = reasoning from proven data, **[hypothesis]** = needs the game.
Builds on `docs/plan/decision-chaining.md` and the speed-up study (session scratchpad `speedup\speedup.md`): the quest graphs
were read from the user's own 5.01 install with scratch tools; extracts and dumps stay in the session scratchpad and are not
in the repo. Only identifiers (block GUIDs, socket names, fact ids, journal GUID paths) are quoted here.

Code: `crates/w3save/src/graph.rs` (`rekey`), `crates/w3al-core/src/branch_swap.rs` (swap file, `OutputSwap::apply`,
`synth_donor`; 4 unit tests), `w3al journal-set --swap FILE`, fixture test
`crates/w3al-core/tests/branch_swap_fixtures.rs` (`W3AL_FIXTURES=C:\w3-fixtures`).

## Short answer

1. **Keira to Radovid built from the Kaer Morhen donor and the graph: PASS** [proven offline]. The synthesized save KR* equals
   the played Radovid donor in the quest log (0 differences, including order, counters and tracking), in the decision facts,
   and in every `questSystem` / `community` node of Keira's and q109's scopes; what remains is the known noise list (section 3).
2. **Letho invited / sent elsewhere: not attempted** (section 5): the choice is made inside one scene (no graph output
   per choice, the facts are set by the scene file), the Letho character-journal listener diverges in shape (a new long wait
   vs the phase ending), and the two available donors also differ by an earlier, diverging decision (fight the headhunters
   or let them go), so no like-for-like reference exists.
3. Side finding [proven offline]: the Keira delta used for gate X1 (`d_keira_km.toml`, 19 ops) leaves q109's **own** thread
   at its pre-decision wait; KR* includes the missing 6 ops (section 4).

## 1. Method

For a decision scene whose outputs **rejoin after instant blocks** (facts, journal, reward, script) before any player wait:

1. Capture output A from a donor as usual (`graph-diff --toml`, `journal-diff`, `facts --diff`) on the template.
2. In the graph, follow output A's and output B's sockets through the instant blocks to the rejoin point, and through every
   listener that branches on the facts they set; list, per difference, A's item and B's item:
   waiting block GUIDs, `CJournalPath` GUID chains of the journal blocks, `factID`s of the fact blocks.
3. Write them as a swap file (`[swap]` with `[[swap.block]]`, `[[swap.journal]]`, `[[swap.fact]]`; format in the
   `branch_swap` module doc) and build with `w3al journal-set ... --swap FILE`. The tool checks the captured delta's guards
   against the real donor first, then rewrites op paths, appended journal entries (+ counters / tracking GUIDs) and fact
   names, and copies from a synthetic donor whose waiting blocks are rekeyed (`graph::rekey`). Every swap item must hit
   something in the delta, otherwise the build is refused (the delta is not output A's).

[hypothesis, shown for one case] A waiting block's saved bytes do not depend on its GUID: output B's block saves exactly
like output A's when both are the same class, entered through the same input socket, with the same number of conditions.
Shown for `f5290e77` -> `c133524a` (KR*'s block equals the Radovid donor's byte for byte); the swap author checks the three
conditions in the graph, the tool cannot.

## 2. Keira (q109): derivation from the graph [proven offline]

Template T_keira `session_V1\ManualSave_568ef_7ea51000_923cb5`, donor KM `..._94abd5`, reference KR `..._97cdc9`.

Scene block `d24f649d` (`q109_wrapup.w2phase` #168, `CQuestSceneBlock`), sockets:
* `Keira to KM` -> `9548cabd` (`CJournalQuestBlock`, input `Activate`) -> `b23ada11` (fact `q109_keira_to_km`, value 1) ->
  `6f75b0f3` (reward `q109_02_persuade_keira`) -> `7159f18a`;
* `Keira to Radovid` and `Geralt Convinced` -> `d16ab830` (`CJournalQuestBlock`, input `Activate`) -> `005478a5` (fact
  `q109_keira_to_radovid`, value 1) -> `cce8d3fb` (reward `q109_02_keira_gone`) -> the same `7159f18a`;
* `7159f18a` -> phase block `f61db075` (q109 thread `bb109b9f`); `Attack` diverges into a fight (not synthesizable).

Listener: `character_journal_entries.w2phase`, phase `a536c75d` (Keira's character entries), pause block `7b32f7d6` with
sockets `keira_to_km` -> `CJournalBlock 8ca6d913` -> pause `f5290e77` (one fact condition, input `In`) and
`keira_to_radovid` -> `CJournalBlock 7fa9eb9a` -> pause `c133524a` (one fact condition, input `In`). The shared part before
it (`b3a72321` socket `did_109` -> `0b775d33`) is the same for both outputs.

| what | output A (KM) | output B (Radovid) | from |
| --- | --- | --- | --- |
| long wait in `143c1d62 > a536c75d` | `f5290e77e94cfd4ca216b846c93d81ac` | `c133524a87776c429ddf03180120419a` | `7b32f7d6` sockets |
| quest objective | `419f978f/ddd5713d/5a772866/7801de377aab6a4381fee5b713690733` | `.../5a772866/fdcc61685ad3364fb611b82ed5a6f23d` | `questEntry` of `9548cabd` / `d16ab830` |
| character entry | `8efa62e1/80e826de/efa7c8fb78127441aa2bb4058e749251` | `.../80e826de/aed9149df836eb4e88a7fc4c8c530276` | `entry` of `8ca6d913` / `7fa9eb9a` |
| fact | `q109_keira_to_km` = 1 | `q109_keira_to_radovid` = 1 | `b23ada11` / `005478a5` |
| reward | `q109_02_persuade_keira` | `q109_02_keira_gone` | `6f75b0f3` / `cce8d3fb` (not modelled, section 3) |

Everything else in the KM delta lies before the scene or after the rejoin point (common to both outputs): q109's own
thread, the shared journal items (`fe6b1b08/ebd1a189` to Success, `fe6b1b08/b47bfb41` + its counter, `keira/7bd54b92`,
highlight cleared), `q109_confronted_keira`, `keira_part1_dd_stop` / `_start`, the default-dialogue scope `973a4357`,
`117e4006 > 7ffa6436`, `ExternalDialog[keira]`, `keira_metz` phase and Keira's stub.

Build (scratch): `journal-set --template T_keira --graph-donor KM --graph-from-toml d_keira_km.toml --graph-from-toml
keira_km_q109.toml --from-toml keira_km_journal.toml --fact q109_keira_to_km=1 --fact q109_confronted_keira=1 --fact
keira_part1_dd_stop=1 --swap keira_to_radovid.swap.toml`, then `edit-fact --remove keira_part1_dd_start` (as gate X1).
Swap log: 1 op path rewritten (`questBlock[f5290e77]` -> `[c133524a]`, 1 donor node rekeyed), 2 journal appends, 1 fact.
Result `scratchpad\synth\ManualSave_568ef_7ea51000_9244b5.sav`: `roundtrip` PASS, `check-save` clean, `journal-set` and
`edit-fact` self-verification PASS. Nothing from the Radovid save was used to build it.

## 3. KR* vs the played Radovid donor [proven offline]

* `journal-diff KR* KR`: **no journal differences** (808 entries, 217 counters, tracking equal).
* `facts KR* --diff KR`: decision facts equal (`q109_keira_to_radovid` = 1, `q109_keira_to_km` absent, `q109_confronted_keira`
  = 1, `keira_part1_dd_stop` = 1, `keira_part1_dd_start` absent). Remaining, all noise or play-session traces:
  record times of the three set facts (we stamp the table's latest time, the game its own clock); scene dialogue-line facts
  `128726629_13_0`, `_16_3`, `_2_0` (the `N_N_N` class: set inside the scene file, not by graph blocks; KM's own
  `128726629_*` are not in the delta either); `mh305_merchant_spawned` (merchant spawn noise, in both donors);
  `tutorial_interaction_container` (a tutorial the Radovid player triggered); load stamps (`*_noticeboard_empty`,
  `InteractiveEntity_empty`, `q105_new_heart_killed`, `RemasterSkillResetAndRefund`, `switch_dual_grip_style`) and the
  `tut_dialog_started` timer.
* `graph-diff KR* KR --block questSystem --block community --stub-id`: Keira's scope `143c1d62 > a536c75d`, q109's thread
  `bb109b9f`, `973a4357`, `117e4006`'s Keira point op and `ExternalDialog[keira]`: **no differences**. Remaining questSystem
  (28): `sq104_villager_1` scene guid, reorders of the scene list and of `b531a276`, `inputName` flips in `4cd34248` /
  `4056cc99` and `bob_quest > 276c5a3b`, timers `5e9d5fdf`, `3eed44ca`, `b4188b3b`, `bob_quest > abead12a`, the mh305 merchant
  spawn handler `83d46257` (`0599050c` -> `c617740e`) and the sq104 day/night phase `a5385375` (`9e4872a1` -> `a70b82d5`;
  world time passed in the donor, not in T_keira). Remaining community (478): 435 `processingTimer`, routine moves
  (position / action point / action category) of unrelated NPCs, `sq104_villagers_blacksmith` Sleep -> Work (day/night),
  the `mh305_merchant` spawn set + its new stub, and Keira's stub `position` / `rotation` (KR* carries where she stood in
  the KM donor at save time; her phase entry, appearance and state equal the Radovid donor's).
* Not compared (outside the four layers): `universe` and the other player blocks. The reward differs by output
  (`q109_02_keira_gone` vs `q109_02_persuade_keira`) and no delta of ours applies rewards; KR* has T_keira's experience /
  inventory, like the KM build [inferred].
* Test: `branch_swap_fixtures::keira_to_radovid_from_the_kaer_morhen_donor_equals_the_played_branch` rebuilds the same thing
  in memory (delta of the two scopes from `graph::diff`, journal from `journal::diff`) and asserts both scopes, the journal
  and the decision facts equal the Radovid donor's.

## 4. Gate X1's Keira delta misses q109's own thread [proven offline]

`chain\d_keira_km.toml` (19 ops; used for gate X1 and the study's `c1_TK_keira` / `c2_TK_keira_letho` builds) covers the
character-journal scope, `973a4357`, `7ffa6436`, the scene registry and the community, but not q109's own thread
`117e4006 > 3673c07f > bb109b9f`. Both donors move it from the pre-decision trigger wait `215f2719` to `d5740aa4` + phase
`f61db075` (thread with `4c2a9b2d`); the study builds still hold `215f2719`. Expected effect in game [hypothesis]: when
the player reaches the tower area again, q109 resumes its pre-decision path. The 6 ops (`graph-diff T_keira KM --block
questSystem --scope <bb109b9f thread>`: remove `215f2719`, insert `d5740aa4`, `f61db075` block + thread, 2 recounts) are in
KR*; the same 6 ops would fix a KM build.

## 5. Letho: why not [proven offline from the graph and the donors' diffs]

* The invite / elsewhere choice is inside scene block `ed477b93` (`sq102_letho.w2phase` #119), which has a single output
  socket `Output`; `q402_invited_letho_to_km` and `q402_told_letho_to_go_elsewhere` are set by no graph block of the 1,041
  graph files (only read: `character_journal_entries`, q403 phases, condition `ff4f19af`). The setters live in the scene
  file, which the scratch reader has not catalogued.
* Downstream, the quest's own path does rejoin: `ff4f19af` (`q402_invited_letho_to_km > 0`) True -> journal `bff46e8b`
  (`903700f4/b1ddb752`) -> phase output `b4ac3133`; False -> the same output. But the listener phase `6850c728` (Letho's
  character entries) branches at `e88d65e8`: `letho_to_km` -> `a03611b3` (`letho/1c0e087b`) -> long wait `7573f7fb`;
  `letho_leaves` -> `a32a9c0a` (`letho/a9539f68`) -> phase output (the thread ends). That is a different frontier shape
  (a wait vs no thread) and one journal item without a counterpart; the swap tool only renames like-for-like items.
* The donors differ by a second decision: scene `70d40ee9` (`Fight` / `NoFight`). The elsewhere donor fought (facts
  `sq102_killed_headhunters`, `actor_sq102_*_was_killed`; journal `6adb31c2/dfba0cf9`, `903700f4/67c71a77`, character
  `letho/29a4c204` via `83c73132` socket `kill`), the invited donor let them go (`sq102_let_headhunters_go`, journal
  `903700f4/4b98797b`, character `letho/ee83d638` via `let_go`). `Fight` diverges into a combat wait (`a4a4f995`, killed
  count) and a trigger wait (`22f54f6c`), so it cannot be made from the other donor; a synthesized "let go + elsewhere" would
  have no played reference to compare with.
* To try Letho: extend the swap with "drop item" and "end thread" items, read the scene's fact setters, and use (or play)
  two donors that differ only in the invite choice.

## 6. In-game check proposed (not run)

Load KR* (10 min): the q109 objective for Radovid and Keira's Radovid character entry show; q109 continues as in the played
Radovid donor (Keira leaves; the quest does not restart the confrontation at the tower); a re-save keeps `c133524a` in
`a536c75d`, `f61db075` in `bb109b9f`, and `q109_keira_to_radovid`. Control: the played Radovid donor `97cdc9`.
Hypotheses this tests: the rekey rule above; the q109 frontier copied from the KM donor works for the Radovid branch.
