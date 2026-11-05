# A content pipeline without a human playing: donor-free synthesis from the game's quest graphs

Date 2026-11-05 (night). Labels: **[proven]** = shown by a test or a command named here on real saves,
**[likely]** = strong evidence, not shown in game, **[hypothesis]** = needs the game, **[estimate]** = my numbers.
Builds on `docs/findings/quest-graph-design.md`, `docs/findings/branch-synthesis.md`, `docs/plan/decision-chaining.md`
and the speed-up study (session scratchpad `speedup\speedup.md`). Code: crate `crates/w3graph` (bundle + CR2W readers,
quest-graph model, walker, save-side frontier/compare, synthesis), CLI `w3al synth-walk | synth-apply | graph-lookup`,
tests `crates/w3graph/tests/synth_fixtures.rs` (`W3AL_GRAPHS` or `W3AL_INSTALL`, `W3AL_FIXTURES`). Nothing from the game files
is in the repository; the crate reads the player's own install (read-only) or a folder of extracted graphs at run time.

## Short answer

1. **Rank 1, built and validated offline: donor-free synthesis.** From a template save and the game's own quest graphs we
   compute what the game writes when the player takes a decision output: the facts, the journal entries, the spawn-set phases
   and the new waiting blocks (the frontier), and we write them into a copy of the template. Checked against four saves a human
   really played from the same templates (Keira to Kaer Morhen, Keira to Radovid, Whispering Hillock released, destroyed):
   **every `questSystem` block the played save gained or lost in the quest's files is predicted, no extra, no miss (9/0/0, 9/0/0,
   34/0/0, 12/0/0 blocks); the journal is predicted with no miss and no extra (5, 5, 14, 13 items) and the synthesized save's
   journal equals the played one entry for entry (0 differences); the decision facts are equal** [proven offline]. What the
   graph cannot give: facts set inside scene files (`q105_betray_spirit`), script side effects (`q105_new_heart_killed`), the
   objective counter the game adds for a new objective, the community (spawn-set/stub) edits, and the runtime state of a
   scripted-actions block that already ran. Per decision the author supplies three things read off the graph: the blocks to
   fire (the template's waits), the output socket to take, and where to stop.
2. **Rank 2, designed, not run: the game as an unattended runner.** The 5.01 scripts expose everything a runner needs:
   `LoadGameInit`/`LoadLastGameInit`, `SaveGame`, `RequestExit`, the dialogue-choice hook (`OnDialogChoicesSet` +
   `SendSignal(SSST_Accept, i)`), `OnGameStarted`, teleports, time. Driver = facts in the save (a proven channel), output =
   `scriptslog.txt`. Draft mod and README: `C:\w3-out\auto_runner` (not compiled, not launched). Its value is the in-game
   smoke test of every generated save and donors for the few branches synthesis cannot build (fights); it does not remove the
   engineering of option 1, and the first launch needs the user (permission, save-folder arrangement, compile check).
3. **Order:** finish option 1 (community from the graph, scene-file facts, counters, questbook integration), gate it in game once
   (K3: a synthesized Keira KM next to the donor-built K2), then build the runner as the test harness. Hours the user still has
   to spend after that: one 10-minute load per gate, no play [estimate].

## 1. Ranking by user-hours saved per engineering day

| | user hours saved | engineering | hours/day | state |
| --- | --- | --- | --- | --- |
| 1. donor-free synthesis (walk the graph, write the save) | all donor play for scene-output decisions: ~25 branches x 10-30 min + the continuation play of windows (~20-30 h, from `speedup.md` 6) | 2 days done; ~5 more (section 5) | ~4 | validated offline on 4 branches |
| 2. unattended runner mod | the human's 10 min per gate (~3 h over v1) and the donors for fight branches (~3-5 h) | 3-5 days + a first supervised launch | ~1.5 | APIs found, draft written |
| 3. console + capture assistant (speedup.md 1-2) | ~10-20 h on play that remains if 1 is not done | 2-4 days | ~4 (but 0 once 1 exists) | superseded by 1 for scene decisions |
| 4. full consequence simulation (play whole quests offline) | would save ~20 h | weeks (combat, triggers, NPC runtime) | < 1 | not for v1; option 1 already walks through single-output waits in `--through` mode, so short tails cost nothing |

Why 1 wins: the walker turns the quest graph into the save state in seconds, the output is deterministic and diffable against
the few played saves we have, and every rejoining AND every short-tail diverging branch (the mq0005, Keira, spirit shapes) is
covered without play. The study's own rejoin count (202 of 994 multi-output scenes rejoin before a wait) is no longer the limit:
diverging outputs are walked to their next waits and those waits are written.

## 2. Option 1: what is generated from the game data, and what is not [proven unless marked]

Reader: `crates/w3graph/src/bundle.rs` (`POTATO70`: 304-byte entries, zlib/stored) and `cr2w.rs` (`CR2W` v163/164: ten
12-byte table descriptors at 0x28, names, imports, exports with `{u16 name, u16 type, u32 size}` property streams,
`CVariant` for script parameters). Original code; the layouts were read from the files, not from WolvenKit. The crate loads all
1,041 `.w2phase`/`.w2quest` of the 5.01 install in 2.4 s from a folder and 3.9 s from the bundles (`graph-lookup --install`),
176,469 blocks; every frontier GUID of T_keira (2,812) and T_spirit (2,973) resolves.

Model (`quest.rs`): a block = GUID, class, file, label, output sockets -> (target block, input socket), owner phase, and the
payload the walker needs: `CQuestFactsDBChangingBlock {factID, value}`, `CJournalQuestBlock {questEntry}` / `CJournalBlock
{entry}` (the `CJournalPath` chain = the save's entry path with its `.journal` resources), `CQuestPauseConditionBlock
{conditions}` (fact conditions with compare function and value, logic operations, real-time delays, everything else by class),
`CQuestConditionBlock`, `CQuestPhaseBlock` (embedded graph or imported file), phase input/output sockets,
`CQuestStoryPhaseSetterBlock {activate (spawn set, phase) | deactivate}`, rewards, `CQuestScriptBlock {function, params}`.

Walker (`walk.rs`): activations `(block, input socket)` are followed through instant blocks; the walk stops at waits. Rules that
the played saves confirmed, each learnt from a miss in the comparison and now in the code:

| rule | evidence |
| --- | --- |
| a phase output ends the phase: its `questThread` and every wait inside leave the save; its `questBlock` leaves too unless the phase is re-entered | spirit: `723f020d` with 5 parallel phases and 15 waits gone in both branches; Keira default dialogue `973a4357`: exit by `Out` -> empty phase -> `Restart` input: wrapper stays, `inputName` = `Restart`, old waits gone, `ad92bd6b` waits |
| a pause block with several conditions has one output socket per condition, in order; the true one fires | `7b32f7d6` keira_to_km / keira_to_radovid / keira_kaput; `b3a72321` did_109 / skipped_109 |
| fact listeners anywhere in the template's frontier fire when the walk's facts make their condition true (gate J2 showed the game does this on load); a listener already true in the template that still waits there is left alone | KM: `b3a72321`, `3552699c`; spirit: `c14d7770`, `f1889334`, `059fd2dc`'s phase; the "already true" rule removed 7 false positives (sq104, mq3009, mq3031 listeners) |
| a listener runs only to its next wait (the player did not play on); a real-time delay of at most a minute counts as over | KM: `c5ce4482` (1 ms) passed, `f5290e77` kept waiting |
| `CQuestCutControlBlock` fires `Out` at once, stays in the frontier (`CutControlBlock {wasOutputActivated = false}`), and delivers `Cut` to the blocks on its other sockets, which leave the frontier | KM: `d5740aa4` waits with input `In`; spirit: `1171cb29`'s cut removed the Soltis dialogue `b9ea9a8e` |
| `CQuestScriptBlock` `RemoveFactQuest(factId)` / `ResetFactQuest` remove a fact (2,102 uses in the graphs); `TryToAddUniqueFact` adds one | KM: `keira_part1_dd_start` removed by `a06aa525`, which also keeps the restart listener `ad92bd6b` waiting |
| a quest root set to Success flips its Active entries (also ones appended in the same walk) to Success | lukwraith 21 -> 22 (5 flips), spirit released/destroyed (4 each) |
| a file imported by several phase blocks: the running importer (a template scope) is the parent; none or several running = unresolved, not followed | `empty_phase.w2phase`, bug-fix phases: hundreds of importers; the first version of the walker leaked into sq104/mq3009 through them |

Save side (`frontier.rs`, `synth.rs`): the template's frontier (every `questBlock` with its scope chain and saved
`inputName`, the running `questThread` wrappers, the `ExternalDialog` registry) is read from `questSystem`; the synthesis
removes fired blocks and ended scopes, inserts fresh waits and scope wrappers built from the layouts documented in `synth.rs`
(`questBlock {GUID, inputNamesCount 1, inputName, activationState 1 [, conditions {numConditions, p {nP, active[, isFulfilled]}}
| CutControlBlock | treeState {numSaved 0}]}`, `questThread {GUID, questThread {numBlocksToActivate 0, numBlocks, ..,
numThreads, .., suspendedScopesData 00000000, pausedThread false}}`), recounts, drops registry entries whose scene block no
longer waits and adds entries for new interaction waits (tag = the block's first actor tag), applies the journal edit (status
flips, appends with the `.journal` resources from the graph, Success propagation) and the facts (set to template sum + the
graph's value, removals), renumbers, rebases, re-reads. `nP = 2` + `isFulfilled` belongs to scripted conditions
(`W3QuestCond_*`), `nP = 1` to the rest (1,522 condition records of T_keira).

### Validation against played donors (session V1, `w3al synth-walk --donor` and `synth-apply --donor`)

| decision (template -> donor) | walk arguments | prediction vs donor: blocks hit/miss/extra, facts, journal | synthesized save vs donor |
| --- | --- | --- | --- |
| Keira to Kaer Morhen (T_keira -> `94abd5`) | `--start 215f2719 --through --pick "d24f649d=Keira to KM" --stop 4c2a9b2d` | 9/0/0; 4/0/0 (3 set + 1 removed); 5/0/0 | `questSystem`: 0 changes in the four quest scopes `bb109b9f`, `f61db075`, `a536c75d`, `973a4357`; journal 808 entries, 0 differences; facts equal. Remaining: the registry entry of sq104 (world noise), 1 objective counter the game added |
| Keira to Radovid (-> `97cdc9`) | same, `--pick "d24f649d=Keira to Radovid"` | 9/0/0; 4/0/0; 5/0/0 | same as above (`c133524a`, `fdcc6168`, `aed9149d` right) |
| spirit released (T_spirit -> `900fd9`) | `--start c2549952 --start c4d54c91 --through --pick ba20c454=q105_ritual_done_out --stop 51f0f429 --stop 0d3a7efd` | 34/0/0; 4 ok / 4 miss (`SignToggled`, `q105_new_heart_killed`, 2 merchant facts) / 0 extra; 14/0/0 | journal 0 differences; every GUID the donor waits at under `0edad8e4` is there and nothing more; only `0d3a7efd`'s `treeState` differs (the scripted actions had run in the donor: `numSaved 1` + entity state) |
| spirit destroyed (-> `91a5f1`) | same, `--pick ba20c454=q105_betrayed_out --stop 1f5f6045` | 12/0/0; 4 ok / 5 miss (+ `q105_betray_spirit`) / 0; 13/0/0 | journal 0 differences; scopes equal |

Written files: `roundtrip` and `check-save` clean (scratchpad `nohuman\out\synth_*.sav`, not for the game yet). The fixture
test asserts all of this (`synth_fixtures.rs`, 3 tests, ~7 s).

#### Re-validation after the gaps were closed (2026-11-05, later; `synth_fixtures.rs`, 5 tests, ~15 s with `W3AL_INSTALL`)

The three gaps below are written now (community from the `.w2comm` files, scene-file facts from the `.w2scene` files,
objective counters from the `.journal` files), so the same four donors plus Letho invited (session V2) read:

| case | journal (entries / counters vs donor) | facts miss / extra | `questSystem` in the quest scopes | community in the touched sets |
| --- | --- | --- | --- | --- |
| Keira KM | 0 / 0 (1 counter `b47bfb41` added, as the donor) | 0 / 0 | 0 changes in `bb109b9f`, `f61db075`, `a536c75d`, `973a4357` | phase `q109_island_laboratory -> q109_island_attacking`, old stub removed; the donor's one extra stub is the new phase's (the game created it) |
| Keira Radovid | 0 / 0 | 0 / 0 | 0 | same |
| spirit released | 0 / 0 (1 counter) | 0 extra; misses `q105_new_heart_killed` (script), `SignToggled`, 2 merchant facts (play) | `0d3a7efd` `treeState` only (ran in the donor) | 8 sets added, `q105_evil_heart` + `nml_burrows_soltis` removed with 2 stubs and 2 `entityData`; donor's 10 extra stubs are the new sets' (game-created) |
| spirit destroyed | 0 / 0 (1 counter) | 0 extra; `q105_betray_spirit` now set (scene section `#90`); same 4 play misses | 0 | 2 added, 2 removed, 2 stubs; donor's 4 extra stubs game-created |
| Letho invited (T_letho -> V2 `a4841f`, `--start 2da9cd80 --through --pick 70d40ee9=NoFight --scene-pick "ed477b93=sq102_go _to_km_letho"`) | 0 added / removed, root entry unread vs read; 0 counters | 0 / 0 (`q402_invited_letho_to_km`, `sq102_letho_completed`, `sq102_let_headhunters_go` all from the walk); values differ: `sq102_hide_community` 2 vs 0, `sq102_explode_building` 1 vs expired | `e9e84a64` And block byte-equal (`ActivatedInputs` `In 1`); `5f88ff5b` (a fact wait the `through` walk passed, add it to `stop` to match) and the `persistent_content` listener the donor played past | 0 differences (Letho, headhunters, boy, horse sets off; 3 stubs + 3 `entityData` gone) |

Walker rules learnt from these (in the code, each with a unit test): W-1 a phase exit drops the activations still queued
inside it; W-2 the journal input `Failure` is `JS_Failed`; W-3 a phase entered inside a phase that ends afterwards is gone
with it (and a cut-control block inside); a `CQuestAndBlock` fires only when every connected input has been activated,
the template's saved inputs counting (`inputNamesCount`, `ActivatedInputs`); the old content of a re-entered phase is not
reused as a path for its new waits. D16 lodge (pack 51) and D11 refuse (pack 35), which W-3 blocked, write.

#### A synthesized window template: W6 Brothers in Arms (2026-11-05, later; `docs/research.md` same date)

No save in hand is inside q402_ciri's "gathering allies" phase `b74543d1` with no ally asked (298 saves scanned), so the W6
template is itself synthesized from pack 43 (early in q401) by the `done_effects` of the quest before it
(`questbook/05_kaer_morhen/ugly_baby.toml`: start `6e852741` [q401_everybody_returned > 2], `--through`, 11 picks along the pack
line, 15 stops = the ally dialogues of `b74543d1` and the island conditions). Brothers in Arms `requires = ["ugly_baby"]`, so
export walks the template first and the four invitations after it, from pack 43 every time; `w3al synth-apply` with the same
arguments writes the template as a file for checks. That is the questbook's way of saying "this window's template = an earlier
save plus preparatory steps": no new schema field, the previous quest's completion is the preparation.

| case | checked against | result |
| --- | --- | --- |
| template (q401 ended, q402 started, nobody asked) | pack 44 old-gen `ManualSave_6b4a7_7e10d400_55c9e52` (all four invited) | every q401 / Part 1 block pack 44 lost removed; q402 scope differs by the allies only; q401_completed, chapter1_completed hit; misses = the skipped searches' play facts and pack 44's Crach / Emhyr |
| all four invited (Ermion `4075a745`, Hjalmar via Cerys `0cd4179d` + `c9e4e916`, Roche `4a92523c`, Zoltan `22f5025a`) | pack 44 | the eight decision facts equal (`q402_invited_*_to_km` x4, `q402_allies_gathered`, `q402_cerys_sent_hjalmar`, `q402_cerys_refused_km`, `q402_hjalmar_invited_to_q403`); 1 extra (`q402_roche_helps_context_dialogue`, 5.01 scene); q402 journal roots and phases present, 17 status / unread differences all from Crach, Emhyr, Dijkstra and reading |
| exports: template, all four, none, Ermion + Roche, Hjalmar + Zoltan | export's own verification | all VERIFIED; the exported template equals the `synth-apply` file byte for byte in `questSystem` and `community` |

Walker rules learnt (unit tests each): **W-4** a file imported by two phase blocks in a row is walked once per importer (the
`visited` dedup was global; the second Unmount Weapons of q401 did nothing); **W-5** a wait assumed satisfied yields to the instant
blocks queued with it (what follows the wait goes to the bottom of the work stack: Roche's output activates a despawn chain and a
journal block at once), starts fire one after the other and a start whose phase an earlier start ended does not fire (spirit donor
`c4d54c91`); **W-6** a phase re-entered through another input socket runs its content again (Cerys `ed29bf82` `First` -> `To_q208`
-> `Second`), the same socket twice keeps the old visits (one lap per socket). Journal: activating an objective of a quest the log
does not know creates the quest's root and phase entries as Active (pack 44; `synth::journal_edit`). The journals of the two search
sub-quests the start skips (`q401_hunt_with_eskel`, `q401_phylactery`) are closed with tables captured by `w3al journal-diff`.

### What a played save still has that the graph does not give

| gap | seen as | fix | days |
| --- | --- | --- | --- |
| ~~**community**~~ closed 2026-11-05: `community::apply_spawn` writes the `c[<file>.w2comm]` phase switch (and `isActive`), builds a new `c` block from the `.w2comm` tables / phases when the save lacks the set, removes a deactivated set with its stubs (swap-remove by idTag) and their `entityData`, and removes the stubs of a phase switched away from. Stub creation is left to the game [hypothesis: the game spawns stubs for an active set/phase on load; gate K3 tests it]; the donors' only community differences are those stubs | | | done |
| ~~facts set inside scene files~~ closed: `scene.rs` reads `.w2scene` (`AddFact` / `RemoveFact` events, `CFactsDBChoiceMemo`, `AddFact_S` / `RemoveFact_S` scripts, flow conditions, choices) and `Scene::outcome` sorts the facts on the way from the input to the output taken into certain and maybe (behind an unpicked choice); `--scene-pick GUID=section` / questbook `scene_pick` names the line. `q105_betray_spirit` and `q402_invited_letho_to_km` are predicted | | | done |
| ~~objective counters~~ closed: `journal_edit_with` adds `count 0` for every appended entry that is a `CJournalQuestObjective` in the `.journal` resources (Keira `b47bfb41`, spirit `bf144fbd`; Letho's new entries are description entries: none). Counters equal the donors' in all 5 cases | | | done |
| scripted-actions blocks that already ran in the donor (`treeState` with entity data) | 1 block, released branch | park before them (choose the stop earlier) or let the game run them from `numSaved 0` [hypothesis] | 0 |
| script side effects (`q105_new_heart_killed`, rewards, layers, map pins `QuestMapPinStates`) | fact misses, untouched blocks | rewards/pins: the game applies them when the block fires, so only visible-state cosmetics are missing; leave | 0 |
| listeners on world conditions (triggers, tags, timers) that the donor's player satisfied by moving around (`7ffa6436`, sq104 day/night, `mh305` merchant) | "other" lines in the comparison | nothing: these happen in play after the load | 0 |

## 3. Option 2: the game as an unattended runner [likely for the APIs, hypothesis for the whole]

Found in the 5.01 scripts (file and line in `C:\w3-out\auto_runner\README.md`): `theGame.ListSavedGames` +
`LoadGameInit(info, false)` (the pattern of the `StartNewGamePlus` exec function), `LoadLastGameInit(false)`,
`SaveGame(SGT_Manual, -1)` + `AreSavesLocked()`, `RequestExit()`, `CR4Game.OnGameStarted(restored)` as the post-load hook,
dialogue choices via `CR4HudModuleDialog.OnDialogChoicesSet(choices)` with `SSceneChoice.description` and
`theGame.GetStorySceneSystem().SendSignal(SSST_Accept, index)`, skipping via `SendSignal(SSST_Skip, 0)`, teleports and time
cheats (mod_tools notes), timers on the player, `LogChannel` to `scriptslog.txt` with `-debugscripts`. No API reaches
`questSystem`; no command line auto-loads a save; scripts cannot read files, so the driver is facts written into the test save
(proven channel) and "the newest save in the folder" is the one to load. The draft mod (`modW3ALRunner`, ~120 lines, annotations
`@wrapMethod`/`@addMethod`/`@addField` that the 5.01 exe lists) loads, waits, teleports, picks dialogue options by index, saves,
quits. Unknowns, in order: compile (no compiler here; the game shows an error box), whether `OnConfigUI` runs at the main menu
without input, whether scripts tick while the window is unfocused, timings (start 20-40 s, load 30-60 s [estimate]), a
dedicated save folder (junction or a second Windows user; the user must agree), Steam Cloud. Launch via Steam with
`--launcher-skip -debugscripts`. What it buys: every generated save gets a load + N seconds + re-save with no human, and our
tooling diffs the re-save as it diffs the user's re-saves today; donors for fight branches by scripted picks + `god`/`killall`.

## 4. Other findings

* `w3al graph-lookup GUID..` annotates any `graph-diff` output with file, class, label, conditions and sockets: the attribution
  work of `decision-chaining.md` (sibling donors, null saves, 98 pack pairs) is now a lookup.
* `synth-walk --show-frontier [text]` lists what a template waits at, with conditions: the "before" save of a decision is found
  by looking for the decision scene's predecessor waits (Keira: `215f2719`; spirit: `c2549952`/`c4d54c91`).
* The walker's `--through` mode walks single-output waits as satisfied, so the window end is chosen by `--stop`; a multi-output
  wait without a pick, a condition it cannot evaluate, or an ambiguous importer is reported as `??` and `synth-apply` refuses
  to write. Letho (choice inside the scene file) walks as far as the scene; the scene-file facts are the gap above.

## 5. Next steps

1. **Gate K3 (user, 10 min; build 0.5 day):** `synth-apply` Keira to Kaer Morhen from T_keira with the community written from the
   graph (step 2 below) next to K2 (donor-built). PASS = same behaviour as X1/K2: Keira gone from the tower, journal shows KM, no
   replay; re-save keeps `f5290e77` and `4c2a9b2d`/`d5740aa4` move on as in the donor. Also S3 = spirit released synthesized,
   on T_spirit, next to the played donor.
2. ~~**Community from the graph**~~ done 2026-11-05 (`community.rs`; validated on the five donors, section 2).
3. ~~**Scene-file facts**~~ done (`scene.rs`, `--scene-pick` / `scene_pick`; Letho and the spirit validated).
4. ~~**Counters**~~ done (`resources.rs` `.journal` objectives; equal to the donors' in all five cases).
5. ~~**Questbook integration**~~ done (`[effects.<key>.synth] start, through, pick, stop, scene_pick`; every synth effect of the
   questbook on a 5.01 template exports and verifies, `docs/research.md` 2026-11-05 late).
6. **Runner (3-5 days, after 1-5):** compile check of `modW3ALRunner` with the user, then the launcher/watcher in `w3al`
   (`w3al run-test SAV --wait 60`), then batch smoke tests of every exported branch.
7. **Gate W6 (user, 10 min):** load the all-four export built from pack 43 (`ugly_baby` + `brothers_in_arms`, section 2) and
   check that Ugly Baby is done, Brothers in Arms is active with the four allies invited (council scene of q403 later), and that
   the synthesized quest roots in the journal (`q402ciri` and the ally quests) look as the game writes them. The same save with
   no ally invited checks that the four dialogues can still be played.

Unlocked first: the Velen window W1/W2 decisions (spirit released/destroyed, Keira KM/Radovid) need no donor any more; their
played donors become regression fixtures. Keira killed (`Attack` -> fight) and Letho invited/elsewhere (scene-file facts) wait
for steps 3 and the runner.
