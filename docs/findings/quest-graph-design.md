# Quest graph (`questSystem`): what a quest's completion touches, transplant, template strategy

Date 2026-11-05. Builds on `quest-state.md` and `journal-editor-design.md` (gate J: journal + facts show a quest as done, but the
stale graph re-runs it). Code: `crates/w3save/src/graph.rs`, `crates/w3al-cli/src/graph_cmd.rs`, graph options of `w3al journal-set`,
tests `crates/w3save/tests/graph_fixtures.rs`. Labels: **[proven]** = checked on real saves by a test or a diff shown here,
**[inferred]** = reasoning from proven data, **[hypothesis]** = needs the game (gate G, `C:\w3-out\gate_G\README.txt`).
Evidence: the 45 consecutive lukwraith saves, and the user's own play saves from gates J1/J2 (`C:\w3-fixtures\after_gate_J*`), which
are a second, independent completion of the same quest from the same starting graph.

## 1. What `questSystem` is made of [proven]

```text
BS questSystem
  VL count:Uint32
  BS questExternalScenePlayers
    BS CQuestExternalScenePlayer            (2: the second is always empty in these saves)
      VL tagsCount:Uint32                   == number of ExternalDialog children
      N x BS ExternalDialog {VL tag:CName (NPC tag), VL dialogsCount:Uint32, VL guid:CGUID}
  VL numQuests:Uint32 = 10
  10 x BS quest
    VL fileName:String                      "quests\witcher3_quest.w2quest", "dlc\bob\...\bob_quest.w2quest", ...
    BS questThread                          (the root thread; recursive from here)
      VL numBlocksToActivate:Uint32
      VL numBlocks:Uint32                   == number of questBlock children
      N x BS questBlock {VL GUID:CGUID, VL inputNamesCount, VL inputName:CName ('' / In / Input / angry / axiied ..),
                         VL activationState:Int32, optional BS conditions {numConditions, p {nP, OP active:Bool}, timeRemaining}}
      VL numThreads:Uint32                  == number of questThread children
      M x BS questThread                    a "scope" wrapper: {VL GUID:CGUID = the questBlock (phase) it runs, BS questThread (inner, as above)}
      VL suspendedScopesData:array:2,0,SQuestThreadSuspensionData
      VL pausedThread:Bool
```

* The tree is the **live frontier**: blocks that are waiting (conditions, scenes, pauses) and the phase scopes that contain them.
  A block that fired is gone; finished quests leave no record. Not every waiting block belongs to a started quest: mq0005's own
  scope `38424c39` exists from save 03 to 37 (the whole White Orchard stay) and waits at two blocks for 18 saves before the quest
  has any journal entry (first in save 21). So a quest that is "not started" in the journal is often already a waiting subgraph.
* `ExternalDialog` = registry of scene blocks that wait for the player to talk to an NPC tag: **every `ExternalDialog.guid` is the GUID
  of a `questBlock` in the same save's frontier** (101 of 101 saves). mq0005: tag `mq0005_arsonist`, guid `300f579b` = the scene block in
  thread `3446e000`; after completion the smith's scene `prologue_smith` / `f2bff115` appears.
* Count rules (`graph::COUNT_RULES`): `numBlocks`, `numThreads`, `tagsCount`, `community > communities > numCommunities`, `community > stubs >
  numStubs` each equal the number of such children: 101 of 101 saves, 112,536 checks.
* GUIDs are graph-node ids of the `.w2quest` resources (stable across playthroughs, not mapped to quest names in the save). Without the
  game's resources a block is tied to a quest only by evidence: an `ExternalDialog` tag, the save pair in which it appears/disappears,
  or two independent completions changing the same nodes.

## 2. The arsonist quest (mq0005), lukwraith 21 -> 22 [proven]

`w3al graph-diff 21 22` gives 34 `questSystem` changes in 8 scopes. The user's own completion from J1 (`gate_J` J1 -> `after_gate_J/
ManualSave_8559a_7ea50c00_5f2c555`, same branch: Axii, Napp brought to the smith; dialogue facts `24353285_2_0`, `24353285_5_1` in both)
gives 19 changes. Their intersection is the quest's delta (paths shortened; `Q` = `quest[witcher3_quest.w2quest]/questThread/questThread[7b125918]/questThread`):

| Scope | Before (21) | After (22) | Role |
| --- | --- | --- | --- |
| `Q/questThread[f0b52e98]/../questThread[b4e73851]/../questThread[38424c39]` (the mq0005 phase) | 3 blocks: `ce70ee7f` (empty thread), `62a65bce` (2 condition blocks, since save 03), `3446e000` (thread: scene block `300f579b` = Napp's dialogue + condition `6f50331f`) | `ce70ee7f`, `7fd9eeae` (one condition block `37614a89`). In the user's completion: `4f5f58fc` (condition `e922b089`) | **the quest itself**: the Napp scene and its parallel branch leave; an epilogue wait remains |
| `questExternalScenePlayers` | `ExternalDialog[mq0005_arsonist]` (guid `300f579b`) | removed; `ExternalDialog[prologue_smith]` (guid `f2bff115`) added | scene registry |
| `Q/questThread[988006f0]/../questThread[55d4b37b]/../questThread[83ca614d]` | condition block `fd4ddac0` (waiting since save 03) | `fd4ddac0` fired: new thread `63dc5c18` with condition `e6cbd808` and the smith's scene `f2bff115` | a **listener in another branch of the graph** (the smith's village scenes; same scope holds the inn peasants' scenes `e19d039b`/`ece00dbd`) |
| `Q/questThread[988006f0]/../questThread[089c7277]/../questThread[408d8753]` | condition block `080f5fdd` (since save 08) | removed (thread now empty) | another listener |

The other four changed scopes of 21 -> 22 (`7456b86a`, `32e7a3d9`, `089c7277/379c64f5`, `c4bba6fb`) do not change in the user's
completion; they are lukwraith's other play between the saves (save 22 is titled "Frying Pan, start"). `inputName` flips (`''` <-> `In`) on
unrelated blocks and a reorder in `8e15d30f` are noise.

Facts advance listeners, not scenes. The save the user made right after loading J2 (journal + six `mq0005_*` facts; graph still 21's)
already has `fd4ddac0` fired (smith scene registered), `62a65bce` and `6f50331f` gone, but **`3446e000` with the Napp scene still waiting**.
So condition blocks that wait on facts fire on load from fact edits alone (answers quest-state.md unknown 2: yes); scene blocks that wait
for the player do not, and that is what reopened the quest in J1/J2.

Other blocks: `community` loses the spawn set `c[mq0005_arsonist.w2comm]` (`numCommunities` 43 -> 42) and the arsonist's stub (`numStubs`
20 -> 19) in both completions; lukwraith's 22 also drops `c[prologue_smith.w2comm]` (not in the user's, so not part of the quest).
Stubs are removed by **swap-remove**: the last stub takes the removed slot (both completions; our `swap-remove` reproduces the user's
`community` structure exactly, only positions/rotations/state of other NPCs differ). `CCommonMapManager > QuestMapPinStates` drops the pin of
objective `b1b2703e`. `universe` changes 853 nodes (mostly opaque entity blobs, not attributable). `gameProp`, `timeManger`, `StrayActors`
are clocks.

## 3. Transplant offline [proven]

`graph::transplant(template, donor, ops)` with ops `replace | remove | swap-remove | insert | recount` by path, built from a diff by
`ops_from_diff` (optionally limited to scopes) or written by hand; guarded by subtree hashes in the TOML.

* **Whole blocks:** replaying the full `questSystem` + `community` diff of every consecutive lukwraith pair on A gives B's two blocks
  **byte for byte on 86 of 86 pairs** (`consecutive_graph_diffs_replay_byte_exact`); 06 -> 07 (twice, duplicate set) is refused because B's
  name table is not A's + appended names.
* **One quest:** the mq0005 ops (16 for `questSystem`, 4 for `community`) on 21 make those four scopes and the scene list equal to 22's;
  every other scope keeps 21's state; `community` loses exactly the arsonist's set and stub; the rules of section 1 still hold
  (`lukwraith_21_to_22_mq0005_scoped_transplant`). G1 on disk: `graph-diff G1 22` shows 0 changes under the transplanted paths, the
  remaining 18 are the four non-mq0005 scopes.
* **Other quests** (24 -> 25 Precious Cargo: 18 changes in 7 scopes; 25 -> 26 Missing in Action: 12 in 4; 36 -> 37 Beast of White Orchard:
  30 in 12): every changed scope replays on its own, byte-exact for that scope (`other_quests_replay_per_scope`). Nothing is interleaved at
  the byte level: a scope is a subtree, siblings are keyed by GUID, counts are recomputed.
* **But a quest is not one subtree.** Its delta = its own phase scope + listener blocks in other branches of the graph + the scene list (+
  journal, facts, spawn sets, stubs, map pins). The phase scope may vanish entirely (24 -> 25: mq0002's thread under `b4e73851` is removed) or
  stay with an epilogue wait (mq0005). Leaving White Orchard (36 -> 37) touches 12 scopes including the DLC graph `bob_quest`. Which
  listeners belong to a quest can only be seen by diffing completions; two independent completions agreeing (as for mq0005) is the evidence
  standard used here.
* **Time:** the only time-like field in `questSystem` is `conditions > timeRemaining` on timer blocks: `Float` (5 occurrences in 45 saves)
  or `GameTime` (7; one block `de8555e3`, saves 10-13, value changes between saves, 4 bytes we do not decode). Whether a `GameTime` timer
  is a countdown or a deadline is unknown, so copying a timer block from a donor at another game time may shift it. The mq0005 scopes hold
  no timer. Clocks live in `gameProp` (`EngineTime`, `GameTime`,
  `recentDialogOrCutsceneEndGameTime`), `timeManger`, `community > stubs > processingTimer`, `StrayActors > timePast`, fact record times;
  none needs rebasing for a graph transplant. Fact times: `journal-set --fact` stamps the newest table time (unchanged behaviour).
* **Names:** a transplant requires the template's NM names to be a prefix of the donor's and appends all the donor's extra names (21 -> 22:
  36). Then every index in copied bytes, including opaque values such as `SActionPointId`, means the same name. No remapping is attempted.

## 4. Can GUIDs be mapped to quests without `.w2quest` files?

Only empirically, and it works well enough for capture:
1. `ExternalDialog` tags name the NPC of a waiting scene (`mq0005_arsonist`, `mq0002_merchant`, `mq0001_brother`, ...), and the guid is the
   scene block: that block's scope is the quest's.
2. Lifetimes across consecutive saves (`w3al tree --grep` per save): a scope that appears/disappears in the pair where the journal starts/
   finishes a quest belongs to it; listeners show up as blocks that fire in the same pair in two independent completions.
3. The plain-text scripts cannot help: quest graphs are resources in the bundles; `.ws` scripts only name facts.

Capture workflow (mirrors `journal-diff`):
1. Template T (the quest parked at the decision) and a donor D played FROM T past the quest (so D's names extend T's).
2. `w3al graph-diff T D --block questSystem --block community` and read the scopes; compare with a second completion if one exists.
3. `w3al graph-diff T D --block questSystem --toml <key> --scope <scope> ...` prints `[effects.<key>.graph]` with ops, paths, `before`/
   `after` hashes and the donor stem; community ops (remove the spawn set, swap-remove its stubs, recount) are added by hand
   (`w3al tree T --path community/stubs/a#8` prints a node and its guard hash).
4. Apply: `w3al journal-set --template T --from-toml journal.toml --fact .. --graph-donor D --graph-from-toml graph.toml --out-dir ..`
   (graph first, then journal, then facts; refused if a guard hash or the donor stem does not match).

## 5. Template strategy (recommendation)

| Option | What it needs | Verdict |
| --- | --- | --- |
| (a) per-act templates captured AFTER the quests that should count as done | one real playthrough per act end state; deltas only for quests done differently | **base strategy** |
| (b) delta transplant per quest on a template parked before the decisions | per quest and branch: a donor played from that same template, journal + facts + graph + community deltas | **only for choice-bearing quests, and only from donors of the same template** |
| (c) facts only / journal only | nothing | refuted by gate J (display only) |

Why: (b) works byte-exactly offline (and is gate G), but each delta is tied to its donor: it needs the template's exact frontier (guards),
and two deltas from two different donors cannot both be applied to one template today (each appends its own extra NM names at the same
indices). A product therefore needs, per act: template T with every choice quest parked at its decision, plus one donor per branch per quest,
each played from T. Applying several requires a typed name remap (BS/VL names and types, `CName`/`Enum` values) AND decoding the opaque
values that carry name indices: `questBlock > ActivatedInputs` is an `array:2,0,CName` (258 occurrences in 45 saves); `suspendedScopesData`
is `00000000` on 21,236 threads and holds data on 91; community has `Vector`, `IdTag`, `SActionPointId` with nested typed nodes. That is the
next engineering step if G1 passes.

The other cases:
* **Quests the template never started:** their start subgraph is usually already waiting (mq0005 waited from save 03). Leave them: the game
  offers them as in a fresh playthrough. Faking "completed" for a quest whose graph never ran = transplanting its scope from a donor that
  did it (same as (b)) plus the full journal set (Case B of quest-state.md); listeners elsewhere are the risk.
* **Failed / skipped:** skipped = the template never started it (journal entries can never be removed). Failed needs `JS_Failed`, missing
  from early name tables (`journal-set --allow-new-names`, untested in game) and the graph state of a real failure (a donor that failed it).
* **NPC / world side:** spawn sets (`community > communities > c[<file>.w2comm]`) and their stubs belong to quests by file path and
  are removable; the entity blobs in `universe` (opaque `SS`) are not attributable and stay the template's [hypothesis: the community system
  despawns NPCs whose spawn set is gone].
* **Inconsistent combinations:** listeners make quests interact (mq0005's end fires a wait in the smith's/peasants' scope). A per-quest delta
  carries the listeners that fired in its capture; if another quest's delta also edits that scope (both replace `83ca614d`), the later one
  wins. Rule for the exporter: refuse two deltas whose op paths overlap unless one was captured on top of the other.

## 6. Gate G result and what is still unknown

**Gate G passed for mq0005 (user-reported 2026-11-05, re-saves read in `C:\w3-fixtures\after_gate_G`):** G1 (save 21 + journal + six facts +
the quest's graph ops + its spawn set and stub) loads without a crash, shows the quest completed, Napp is gone from the world, and talking to
Willis does not reopen the quest. The re-saves keep every transplanted node; the only quest-related new fact is the Willis dialogue id
`12721093_1_3`, which the real save 22 also has. J1/J2 had the same journal and facts but 21's graph and reopened the quest, so the graph +
community transplant made the difference. G0 (control) passed; G2 was not needed.

Still unknown:
1. Other quests and branches (only one quest, one branch, a same-playthrough donor are proven).
2. `universe` entity data of a removed spawn set: stayed 21's in G1 and did no visible harm here; not shown in general.
3. Name remapping for multi-donor exports (see 5); cross-playthrough donors.
4. Whether stale `QuestMapPinStates` pins matter (kept in G1/G2).
5. Graph deltas in the questbook schema/lint and in `Document::apply`/export (not done; export still refuses quest effects).
