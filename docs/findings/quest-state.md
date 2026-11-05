# Quest state: facts vs questSystem vs journal (plan unknown 3)

Date 2026-11-04. Read-only research, no repo code touched. Evidence: typed trees (`w3save::Document`) of the 45 consecutive
lukwraith saves (5.01, `saveVersion` 67), name-aware dumps and flat diffs of `questSystem`, `CJournalManager`,
`CWitcherJournalManager`, `gameProp`, `community`, `worldState`, `universe`, `timeManger`, plus the fact diffs. Labels:
**[proven]** = seen directly in the diffs, **[inferred]** = reasoning from the diffs, **[unknown]** = needs an in-game test.
Throwaway tool: `...\scratchpad\queststate\src\main.rs` (commands `top dump journal qs jdiff jbytes factdiff names jmisc`).
Quest identities below come from journal file names (`mq0005dwarfsmith.journal`) and fact prefixes, not from game files.

## Short answer

Facts alone do NOT express a quest state. The visible quest state (Active / Success / Failed per quest, phase and objective)
lives in `CJournalManager > JActiveEntries` (+ small side tables in `CWitcherJournalManager`). Some quests also set a
`*_complete` fact, many do not. `questSystem` is the running quest-graph frontier, not a status table. `gameProp.gameplayFacts`
is unrelated (tutorial/UI flags only).

## 1. Where each piece of state lives

| Place | What it holds | Quest status? |
| --- | --- | --- |
| `facts` | script facts. Quest-ish names are inconsistent: `mq0005_complete`, `mq0004_completed`, `mq0001_completed`, `mq0003_complete`, `th1003_fdb_viper_complete`; but **no completion fact** for `mq0002` (Precious Cargo, 24->25) or for `q002` (Beast of White Orchard, 36->37; only `q002_reward_accepted`, `q002_killed_griffin`) | partial, per-quest accident |
| `CJournalManager > JActiveEntries` | `VL Size:Uint32` + N x `BS SJournalEntryStatus`. Entry = `BS path` (1..4 x [`guid` CGUID, `resource` String, `flags` Uint8]) + `status` EJournalStatus (`JS_Active`/`JS_Success`/`JS_Inactive`/`JS_Failed`) + `JEntryUnread` Bool | **yes, authoritative for the quest log** |
| `CWitcherJournalManager` | `JObjectiveCounters` (Size + `JObjectiveCounter{guid,count}`), `JTrackedQuest{guid}`, `JHighlightedObjective{guid}`, `JHuntingClues`, `JMonsterKnown`, `JEntryAdvancedInfo` | tracking + objective counters |
| `questSystem` | 10 `quest` children, one per `.w2quest` file (`fileName` String). Recursive `questThread` (`GUID`, `numBlocks`, `numThreads`, `numBlocksToActivate`, `suspendedScopesData`, `pausedThread`) and `questBlock` (`GUID`, `inputNamesCount`, `inputName` CName, `activationState` Int32 0/1, optional `conditions` > `p` > `active` Bool, `timeRemaining` Float). Plus `questExternalScenePlayers > ExternalDialog{tag CName, dialogsCount, guid}` | **no quest id / status** |
| `community` | spawn sets (`.w2comm` path, `activePhase` CName, agent stubs). e.g. `mq0005_arsonist.w2comm` `activePhase=mq0005_up_and_about` | world effect, not status |
| `gameProp.gameplayFacts` | 9..12 UI/tutorial flags (`PlayerIsGeralt`, `tutorial_*`, `closingHubMenu`, ...). Union over all 45 saves: 13 names, none a quest fact | no |
| `universe`, `worldState` | entities/spawns/positions/visibility hashes; `worldState` is 18 bytes (`entityStateChangeRequests` count 0) and never changes | no |

### questSystem details [proven]
- File structure: `numQuests` = 10, Q0 = `quests\witcher3_quest.w2quest` (99 KB in save 21) holds the main graph AND all
  side quests as nested threads; Q1..Q9 are DLC graphs (6.8 KB .. 317 bytes) and never changed in any studied pair (except 2 lines in 36->37).
- It is the live frontier only: 882 (save 01) .. 1,114 (05) .. 1,740 (45) blocks, activationState 0 on 41..107 blocks and 1 on the rest, size 95 KB (save 01) -> 121 KB -> 104 KB -> 189 KB (save 45); it
  does not grow monotonically. A finished quest leaves no record of its own: in 21->22 the scene block `300f579b`
  (`ExternalDialog tag=mq0005_arsonist`, guid `300f579b470d8e43...`) and its thread `3446e000`, plus thread `62a65bce`,
  vanish from scope `38424c39`; a new waiting thread `7fd9eeae` appears. Parent scopes stay.
- Block GUIDs are graph-node ids of the `.w2quest`. Without the graph (or a diff anchor like the ExternalDialog guid) a block
  cannot be tied to a quest. Starting a quest by writing blocks is therefore not feasible from the save alone.
- Noise: `timeRemaining` floats on timer blocks (`inputName` "10-30 seconds"), activation of unrelated waiting blocks.

### Journal details [proven]
- Entry order is **pure append**: in all 44 consecutive pairs the old entries are an exact prefix (same order); no entry was ever removed (0 removals in 44 pairs).
  Count grows 98 (save 03) .. 208 (21) .. 318 (36); imz endgame save 6,106 entries.
- `path.flags` = 0 on every element but the last, 1 on the last (214 of 214 entries in save 22).
- Depth 1 = category file (`minorquests.journal`), 2 = quest (`mq0005dwarfsmith.journal`, quest-level entry), 3 = group, 4 = objective;
  child resources are empty strings (same file). All 45 `JObjectiveCounter` guids of save 22 are depth-4 entries.
- JS_Failed appears in later-game saves only (imz 5, hosamnasr 21 entries) and never on a quest-level entry in the sets I have; always on groups/objectives. `JS_Failed` is an NM name that does not exist in early-game saves.
- A completed quest keeps Active children (q002: `4c19caa0` group stays Active, a new `ce689bed` child is added Active; mq0004: `e74de32b/9c8e6fd5` Active) while the quest-level entry is Success. [proven]

## 2. Pairs (what changes when a quest goes started -> completed)

Bytes counted from the typed tree (`jdiff`). `Active`=NM 1524 (0x05f4), `Success`=NM 1837 (0x072d) in these saves (indices are save-specific).

| Pair | Event | facts | JActiveEntries | CWitcherJournalManager | questSystem | other |
| --- | --- | --- | --- | --- | --- | --- |
| 21->22 | mq0005 (arsonist) completed | +`mq0005_complete`, `_arsonist_gets_up`, `_axiid_arsonist`, `_brought_arsonist`, `_stop_hypnosis` | **5 existing entries flip Active->Success** (quest root `0f3305c0`, groups `3eb16b1a`, `f02a0417`, objectives `3eb16b1a/5dd2f404`, `f02a0417/b1b2703e`); each changes only the 2-byte status (`f4 05` -> `2d 07`), size unchanged, `unread` unchanged. +6 appended entries (2 mq0005 end-state: `f02a0417/753019eb`, `3eb16b1a/2734d586`, both Success unread; 4 tutorial). Size 208->214 | counter `753019eb` appended (Size 44->45); `JTrackedQuest` `0f3305c0`->`7c198a76` (q001yennsearch); `JHighlightedObjective` `b1b2703e`->`6b9fcac1` | 53 changed lines (12 blocks added, 15 removed), ~12 about mq0005 | `community` -2 communities (`prologue_smith.w2comm`, `mq0005_arsonist.w2comm`), `numStubs` 20->18 |
| 24->25 | mq0002 (Precious Cargo, spawnset path `mq0002_missing_goods`) completed | **no completion fact** (only `mq0002_merchant_ran`, `mq0002_stop_riding`) | 7 existing entries flip Active->Success (+5 appended, Success). Size 236->241 | - | 52 lines | `community` activePhase `mq0002_at_swamp` -> `mq0002_merchant_at_camp` |
| 22->23 | mq0004 (frying pan) started AND completed | `mq0004_accepted_quest`, `_quest_accepted`, `_completed`, `_met_woman`, ... | 10 entries appended in activation order (phase first, quest-level entry in the middle), final state mixed Active/Success | - | 103 lines (+8/-41 blocks) | `community` -1 community |
| 36->37 | q002 griffin completed | only `q002_reward_accepted`, `q002_inn_brawl_started`, `q002_lashes_count`; **no complete fact** | 7 existing flip to Success (quest root `746b08f6`, `925c6e38`, `63a2e6e3`, ...), +3 appended. Size 318->321 | tracked quest set to the next main quest `7c198a76`/highlight `dde80f16` | 52 lines | - |
| 31->36 | q002 started (31 has no q002 entry) to near end (36) | ~100 `q002_*` facts, `q002_killed_griffin` | 41 appended (q002griffin, q002healingwoman (Success), bestiary spectre entries, ...), pure append; `JTrackedQuest` -> `746b08f6` | - | 102 lines | - |
| 03->04 | start of "Collect Them All"/griffin bestiary | - | +7 appended (bestiary `hybrids.journal`, `griffin.journal`; q001 objective Success) | - | 200 lines | `community` 1,136 lines |
| 05->06 (no quest event) | control | gwint/fist facts | 0 | 0 | **0** | `gameProp` 14, `community` 34, `universe` 14 |

Counts of changed lines (name-aware, size field stripped):
21->22 `questSystem` 187, `CJournalManager` 76, `CWitcherJournalManager` 9, `gameProp` 19, `community` 274, `universe` 593, `worldState` 0, `timeManger` 6;
24->25 `universe` 4,586 (world streaming), 30->31 `CJournalManager` 0.

## 3. Timers and noise (not quest state) [proven]
- `gameProp.EngineTime`, `GameTime` (Double), `recentDialogOrCutsceneEndGameTime` (GameTime); `timeManger.time`, `timeScalePriorityIndexGenerator`.
- `questSystem` `timeRemaining` Float, waiting blocks coming and going.
- `community` `processingTimer`, `spawnDelay`, agent `state`/`teleportToDestination`, `numStubs`.
- `universe`: entity lists, `hash` visibility lists, positions (`Vector`/`EulerAngles` opaque), 600..4,600 changed lines per pair.
- `gameProp.gameplayFacts`: grows with tutorial flags (9->12 entries between 21 and 22), never quest facts.

## 4. Does the journal come from questSystem on load?
Only diffs can speak: **the journal is stored, not derived** [inferred from proven data]. Evidence: (a) the quest graph keeps no
record of finished quests (their blocks leave the frontier) while the journal keeps all of them forever (append-only, 98 -> 6,106
entries), so it could not be rebuilt from `questSystem`; (b) `JEntryUnread`, `JObjectiveCounters`, `JTrackedQuest` are journal-only
UI state; (c) journal GUIDs (journal resources) and quest-graph GUIDs are different id spaces (save 22: 216 distinct GUIDs in the journal, 1,020 in `questSystem`, 0 in common; 0 in common with `community` either).
Whether the game ALSO re-derives or repairs the journal from graph blocks on load is [unknown].

## 5. Minimal edit set for "quest X completed" (proposal, all HYPOTHESES until tested in game)

Case A - quest already started in the template (entries exist, status Active): this is the cleanest, 2 bytes per entry.
1. `CJournalManager > JActiveEntries > SJournalEntryStatus[i] > VL status:EJournalStatus` (value `Enum`/u16 NM index): `JS_Active` -> `JS_Success`
   for the quest-level entry and every group/objective entry of that quest (5-7 entries in the pairs above). Look the u16 up by NAME in the
   target save's NM table; never copy 1837 literally. Before/after bytes of the quest-level entry of mq0005 (186 bytes, only offsets 177..178 differ):
   ```
   4253 ef05 | 4253 1d00 (path)
     564c ce00 cf00 67e5afe3ceb5c148a9cde5c4a99c7a84                     guid  (minorquests.journal)
     564c f005 1100 ab <43 chars>"gameplay\journal\quests\minorquests.journal"   resource
     564c f105 1b00 00                                                   flags 0
     564c ce00 cf00 0f3305c02a250d4aa75c6fba97048763                     guid  (mq0005dwarfsmith)
     564c f005 1100 b0 <48 chars>"...\mq0005dwarfsmith.journal"          resource
     564c f105 1b00 01                                                   flags 1
   564c f205 f305 | f405 -> 2d07 |                                       status  Active -> Success
   564c f505 0500 01                                                    JEntryUnread = true
   ```
2. Append the quest's end-state entries when the template lacks them (e.g. `f02a0417/753019eb`, `3eb16b1a/2734d586`), at the END of `JActiveEntries`, and increment
   `JActiveEntries > VL Size:Uint32` by the number appended (verified: Size == number of entries on all pairs). Source: a donor save that has the quest completed, copying entry nodes verbatim (types `BS SJournalEntryStatus`, `BS path`, `VL guid:CGUID`, `VL resource:String`, `VL flags:Uint8`, `VL status:EJournalStatus`, `VL JEntryUnread:Bool`).
3. `CWitcherJournalManager > JObjectiveCounters`: append `BS JObjectiveCounter{VL guid:CGUID, VL count:Int32}` for newly added objective entries (Size +1 each). [inferred: counters exist only for depth-4 entries]
4. `facts`: set the quest's own fact where one exists (`mq0005_complete` = 1 plus the decision facts). Quests without a completion fact need nothing here.
5. `CWitcherJournalManager > JTrackedQuest`/`JHighlightedObjective`: in the pairs the game retargets the tracker when the tracked quest ends. Probably re-point to the next quest or leave (unknown whether a stale tracked guid matters).

Case B - quest never started in the template: needs the full entry set (and their order of activation) from a donor save, plus the facts; the quest graph frontier (`questSystem`) cannot be authored (block ids unknown). Prefer "mark the whole template pass as completed" over faking half-started quests.

Optional world-effect edits (not needed for the log): `community` (remove the quest spawn-set `c` nodes and decrement `numCommunities`; 21->22 shows `numCommunities` 43->41), `questSystem` (remove the quest's scene/thread blocks, adjust `numBlocks`/`numThreads`, `ExternalDialog` list and its count `tagsCount`). Both need donor data and are the riskiest.

Update (same day): A.1-A.3 and A.5 are implemented (`w3save::journal`, `w3al journal-set`) and reproduce the game's own 21 -> 22 journal
bytes; new counters are hash-ordered (21 -> 22: slot 27), not appended. See `journal-editor-design.md`; unknown 1 below is gate J (`C:\w3-out\gate_J`).

## 6. Unknowns that need an in-game test
1. Does the journal show a quest as completed after ONLY the status flips (Case A.1)? Cheapest test: take a save in the middle of a quest (e.g. lukwraith 21) and flip the 5 entries of mq0005 to Success. Then check the Journal tab, and whether the Napp scene/NPCs (still in `questSystem`/`community`) re-trigger.
2. Does a live-graph block waiting for a fact (e.g. `mq0005_complete`) fire after load if only the fact is set (so facts-only edits may advance the graph)?
3. Is a stale `questSystem` frontier (quest blocks still waiting) harmful when journal+facts say the quest is done? It is what a fact-only/journal-only edit produces.
4. Are appended entries order-dependent (hash/lookup by path vs sequence)? Are `JEntryUnread=true` entries harmless?
5. `JTrackedQuest` pointing to a completed quest.
6. Adding `JS_Failed`/other enum names that are absent from the template's NM table (needs an NM-append routine; none exists).
7. Whether quest-level `JS_Failed` exists at all (not seen at quest level in any of the 70+ saves available).
