# Format unknowns: container header, pointer block, third header int, DLC and mod nodes

Date: 2026-11-04. Read-only research pass; nothing in the repo besides this file was touched.
Data: 77 distinct saves = `C:\w3-extract` (Nexus; lukwraith-8oct is a byte-identical duplicate of lukwraith-26oct and was skipped)
+ `C:\w3-fixtures` (17 own + 4 game re-saves in `after_gate`). 61 are 5.x (`[67,29,164]`), 1 is `[66,29,164]`, 15 are older.
Tools (throwaway, snapshot copy of `w3save`): `...\scratchpad\unknowns\` (`unk.exe hdr|rec|chk|tail|hints|tree|sig|dlc|worlds|selfref|leafdiff|cksum|savetime ...`).
Everything below is measured on those files. "Game accepts" statements come from `research.md` (gates A/B/C/D/T), not from new in-game tests.

## 1. Container header: the "end offset" and the non-zero padding (RESOLVED)

**Answer: the stored last-chunk end offset is a stale value. The game keeps one header buffer in memory and re-uses it for every
save it writes in the same process. A new save overwrites only `(compressed size, decompressed size)` of the records it needs,
and the `end` field of the LAST record it writes is never written. Slots behind the last record are not cleared either.
Whatever the previous multi-chunk save left there stays. The padding bytes and the odd end offset are the same thing.**

Evidence (exact, from the game's own re-saves in `C:\w3-fixtures\after_gate`):
- `ManualSave_8559a_7ea50c00_5a8b7da.sav` (1 chunk, stored end 339092, computed 3084+231963 = 235047) has, right after its one record, 4 stale
  records at file offset 28: `(148757, 1048576, 487849) (145626, 1048576, 633475) (270854, 1048576, 904329) (264297, 465396, 0)`.
- `ManualSave_52587_7ea50c00_5a639fc.sav` (5 chunks, written by the game just before) has records
  `(336008,1048576,339092) (148757,1048576,487849) (145626,1048576,633475) (270854,1048576,904329) (264297,465396,0)`.
  The stale block is byte-identical to slots 1..4 of that file, and the stored end 339092 = slot 0's end = 3084 + 336008.
  `CheckPoint_8559a_7ea50c00_5a89e13.sav` (also 1 chunk) carries the same stale table. A 1-chunk write does not clear it,
  so 5a89e13 and 5a8b7da share the stale value.
- All 7 saves out of 77 with a non-zero last end are single-chunk AND have non-zero padding (7 of 7, no exceptions). All 53 multi-chunk saves have last end 0 and clean padding.
- Every non-last end offset is exactly `3084 + cumulative compressed size`: 213 of 213 records. Every non-last decompressed size is 1,048,576.
- Same pattern in the lukwraith set: save 07 (stored 286585, stale slot 1 = (373, 504, 0)), 30 and 31 (261663, stale slot 1 = (52855, 92660, 0)),
  35 and 36 (316528, stale slot 1 = (14, 13, 0)). Each is a 1-chunk save written after an unseen 2-chunk write (autosave/checkpoint)
  in the same game session. Consecutive saves 30/31, 35/36 and checkpoint/manual share the stale value, which a persistent in-process buffer predicts.
  A new process (hours later, e.g. 27 -> 28) gives clean zeros.
- Rejected hypotheses (numbers checked): payload size, previous file size (07: 286585 vs 06 = 232432), compressed total, a 1 MiB multiple. None match.

Consequences for the writer:
- The last record's end offset is a don't-care/uninitialised field. The game loads files with last end 0 (gate B: `ManualSave_8559a_7ea4d400_4544c16.sav`, 1 chunk, end 0) and
  files with a computed absolute value on the non-last records (gates A/C/D, 5 chunks, last end 0). Recommendation: always write `0` for the last record and zeros for the padding;
  keep non-last ends as `data_start + cumulative` (that is what the game writes, 213 of 213).
- `Container::diagnostics` should report "last end != 0 with non-zero padding" as INFO (stale buffer), not as a failure of the hypothesis. The rule "the end offset is absolute" holds for every non-last record.
- Untested: whether the game ever reads the last end (it evidently does not need it).

## 2. Pointer block `u32 NM, u32 RB, "SC"`, other tags, checksums, size-dependent fields (RESOLVED, negative results)

- `SC` after the pointer block and `SE` in the footer: constant on 77 of 77. Gap sizes between variables are fixed per game generation: 16 (header) + 212 (RB 6+6*34 plus the 2-byte `NM`) on all 61 5.x saves, 206 on 12 4.x saves, 200 on 4.
- Structural tags seen: `SAV3` (magic), `NM`+`MANU`, `ENOD`, `RB`, `BS VL OP SS`, `SXAP`, `SBDF`/`EBDF`, `BLCK`, `AVAL`, `PORP`, `SC`, `SE`.
  New: inside `SS` bodies there are also `STOR` ... `ROTS` (start/end markers, `ROTS` = `STOR` reversed, like `ENOD` = `DONE` reversed): 7,031 pairs in the user's 5.01 save.
  Also nested `SXAP` headers (7,209 in that save's 180 `SS` bodies), not only one per `SS`. Non-facts `SS` bodies are a nested stream with their own markers; do not assume one SXAP per SS.
- Checksums: none. All bytes outside the variables are accounted for (16-byte header, RB, NM tag, pointer block, table, footer) and none is a hash. In addition I computed CRC32, Adler-32, FNV-32 and a byte sum over 12 plausible ranges
  (whole payload, minus footer, minus tail, from 16 or 4, up to NM/RB/table) plus CRC32/Adler of the compressed region and of the whole file, and searched for each result (LE and BE) in the container header and everywhere in the payload:
  0 hits on 77 saves. `saveInfo.magic_number` (14 bytes, differs per save) is not derived from the payload by any of these.
- Payload-size dependent fields not recomputed by `rebase`: none found.
  (a) `selfref` scan: every unaligned u32 in the payload compared with about 40 derived values (payload/file length, chunk sizes, NM/RB/table offsets abs+rel, start/end/size of each top-level node abs+rel, variable count, NM name count, fact count, table bytes; values >= 4096). Outside the known RB/pointer/footer fields only the `MANU` name count (inside `MANU`, its own count) and the `SBDF` fact count (inside the facts SS) hit consistently; the other 531 hits are one-off chance hits.
  (b) Typed-leaf search for the fact count in every variable of saves 01/20/45 (134, 1218, 1992 facts): no other variable holds it (`gameProp.gameplayFacts` is a separate list).
  (c) Pairs with big payload changes: lukwraith 24 -> 25 (payload -310,964 bytes; `universe`/`questSystem` change) differ in only these small-node leaves: `ContainerManagerSaveInfo` SS body (+19 bytes, its own SS length), `TickManager.timers.timerIdGenerator`, `gameProp.EngineTime/GameTime`, `timeManger.timeScalePriorityIndexGenerator`, `idTagManager.tagIndex`, `envManager.weatherConditionName`, plus `saveInfo`. No value there is a size or offset. 09 -> 10 (+756): `saveInfo` only.
  (d) `saveInfo` is a per-playthrough save history, not size data: `magic_number` (14 bytes: `u32 10`, then 10 bytes), `description`, `runtimeGUIDCounter`, `playthroughId`, `count`, then `count` x (`time` u64, `type` u8, `v` u16 = 67). Each save appends one triple (`type` 1 for the first, 2 after); `count` rises by the number of saves made in between, autosaves included (24 -> 25: 234 -> 241). On export we reuse the template's history, which is internally consistent and was accepted by the game.
- Side finding, file naming `ManualSave_<a>_<b>_<c>`: `b` and `c` are the high and low 32 bits of the LAST `time` value in `saveInfo` (`b` equal on 76 of 76 files whose name could be parsed; `c` = low word minus 1..165, typically 10-15).
  `b` is always a multiple of 0x400 and the low word is below 2^26, i.e. the 64-bit number looks like `(bucket << 42) | counter`. When the counter passes 2^26, `b` goes up by 0x400 (lukwraith 06 `7ea4d400_04544c15` -> 07 `7ea4d800_00b2dd10`). That is why they "grow with every save".
  `a` is NOT derived from this (it repeats: 8559a on saves 03-10, 20, 22, 23, 30, 31, 37) and its meaning is unknown (not the quest: 8559a spans several quests). Exporter hint: derive new `b`/`c` from the template's last `saveInfo.time` plus an increment; to be fully consistent also append a triple and bump `count`. The game accepted invented ids, so this is cosmetic.
- Sidecar `.json` vs payload: payload `InitialBuildIDBlock/InitialBuildID` equals the json "initial buildID" on all 5.x saves; "current buildID" exists ONLY in the json. The user's 5.01 re-saves say current = `5.0.1090142 P4CL: 13662125`; Nexus/lukwraith and the user's older 5.x save say `5.0.1048522`; WestAard (`saveVersion` 66) is `5.0.1041720`. All 5.x have header ints `[67,29,164]` (66 for WestAard), so the patch cannot be read from the header ints.

## 3. Third header int, the zero u32 around the name table, `c1` (PARTLY RESOLVED)

- Third int by group (all 77): 164 on `saveVersion` 66 (WestAard, build 5.0.1041720) and 67 (gameVersion 29); 163 on `saveVersion` 64 with gameVersion 18, 25, 26, 27 (all pre-5 saves, 2016-2023); Atvaark's 1.x sample was 162. It does not change with gameVersion 18..27 nor with saveVersion 64/66/67 inside a group, so it is a coarser format/engine-layer version that moved 162 -> 163 -> 164 at generation changes. `SXAP` repeats exactly the same triple. `buildPatch` is 0 in every json. UNRESOLVED: what the engine does with it. Test (safe, in a copy): set it to 163 in a copy of a 5.01 save and try to load; a refusal or "wrong version" path would show it is read.
- The two zero u32 around the names: 0 on 77 of 77 (before the first name and before `ENOD`). Nothing else found; `u32 count, u32 0` fits a 64-bit count whose high half is 0, but that is only a guess. Harmless while kept verbatim (`Manu.unknown_a/b` already are). UNRESOLVED.
- `c1` = number of records with expiry >= 0: already in research.md; not re-tested here.

## 4. The out-of-range "first u16 name index" variables (RESOLVED)

All of them are `SS` entries. `SS` is `tag + u32 length` and has no name index; the first u16 after the tag is the LOW half of the length, which is sometimes above the NM count. Counts: 604 of 18,411 `SS` on the 77 distinct saves (964 if the 45 duplicate files are counted); `BS`, `VL`, `OP`: 0 of 4.7M. The earlier "782" is the same effect on the earlier file set. Nothing to fix; the typed tokenizer already treats `SS` as length-prefixed.

## 5. DLC, NG+ and mod nodes

### Nodes and what they store (values from the 61 5.x saves unless noted)

| Top-level node | Content | Meaning / use |
| --- | --- | --- |
| `AdditionalContent` | `count` u32, then `count` x `name: CName` | Content packs mounted when the save was written: `bob_000_000` (Blood and Wine), `ep1` (Hearts of Stone), `dlc_002_001, 003, 007, 009, 010, 011, 012, 015` (free packs). **The same 10 names are on all 61 5.x saves, even the first Kaer Morhen tutorial save**, so this is "installed", not "played". Order differs on 4.x (ep1 second). Three 2018 saves from the user's own old playthrough have `count 0` and no names: that is what a machine WITHOUT the packs writes. |
| `AdditionalSaveInfo` | `HonorSystem2` bool, `SavedBlackscreen` bool, `QuestLoadingVideo` string, `LoadingScreenName` CName, `ContentTaintedFlags` u32, `content0`..`content16` bool | `contentN` is always a prefix of trues (0..k): k+1 = 2 Kaer Morhen, 3 White Orchard, 4 Vizima, 5 Velen/Novigrad, 8 Skellige, 13 once Hearts of Stone has started (and in Blood and Wine). Reads as "highest content chunk touched" (monotone with progress), NOT as DLC ownership. UNRESOLVED whether the game checks it. |
| `ContentTaintedFlags` | u32 | 12 on every vanilla 5.x save (lukwraith 47, user 4, WestAard). 76 (= 12 + 64) on all hosamnasr saves (6 mods, NG+), on imz (edited; `IsModdedSave` false but the flag is still 76) and on most old user saves. Hypothesis: bit 6 is a sticky "tainted by mods/unofficial content" bit. Not proven. A vanilla template must have 12. |
| `CModHandlerSystem` > `ModNameSaveData` | `IsModdedSave` bool, `Size` u32, then `Size` x {`ModName` String, `ModID` Int64 (-1), `ModVersion` Int32 (-1)} | THE mod node; mirrors the json `modsMetadata`. hosamnasr: `true`, 6, `modnoreload`, `modftfaremastered`, `redkit_modfixstuckquestitems`, `modarkkaniscrowsperchgwentdealer`, `modallskillsactivewithoutslots`, `modallquestmarkersonmapremaster`. Vanilla: `false`, 0, no further children (2 children). Absent on 4.x saves. |
| `NewGamePlus` | `Enable` bool, `active` bool | Vanilla: false/false (lukwraith, user). Hosamnasr and imz: `active` true (NG+ playthrough). Old 11a7d0 (Blood and Wine, NG+ menu): `Enable` true, `active` false. |
| `universe` > `world` x N (+ `count`) | one `world` per level the save has visited | DLC marker for Blood and Wine only: a world `dlc\bob\data\levels\bob\bob.w2w` (hosam 7-9, imz, old 11a7d0 = 9 worlds; absent from all vanilla). `levels\island_of_mist\...` and `levels\the_spiral\...` are base-game worlds (they show up in hosam 5 with 8 worlds, before Blood and Wine). Hearts of Stone has no world of its own. |
| `worldInfo/world` | String | `dlc\bob\...` while the player stands in Blood and Wine. |
| `facts` | five `*_installed` facts: `dlc2_installed`, `ep1_installed`, `ep2_installed`, `mh201_installed`, `sq107_installed` | Permanent value-1 records written by the game's scripts at first load (game time about 1129.57 on lukwraith 01, 52.89 on WestAard, 59.2 on the user's). Present on every 5.x save. The three old 2018 saves with `AdditionalContent` count 0 contain the string `ep1_installed` 0 times; the other old saves contain it. Second marker of "pack installed". |
| facts named `q6xx` / `q7xx` | Hearts of Stone / Blood and Wine quest facts | hosam 4 (before DLC1): 8 `q6xx`, 0 `q7xx`; hosam 6: 796 / 2; hosam 7: 796 / 235; hosam 9: 796 / 1848; imz: 987 / 2032. Lukwraith 01-45 and the user's 5.01 save: 0-2 `q6xx` (`q602_village_noticeboard_empty`, `q603_circus_camp_default` are base defaults), 0 `q7xx`. These are the DLC progress markers. |
| `InitialBuildIDBlock` | `InitialBuildID` String | New in 5.x (absent on 16 older saves); equals the json "initial buildID". |
| `TelemetrySessionId` | old only (12 saves) | 4.x-only node. |

### Mods leave a second footprint in facts
`modRevealMapPOI_*` (428-493 facts) appear only in the hosamnasr set (mod "allquestmarkersonmapremaster"). `lw_*` facts are base game (in every lukwraith save). A mod-free template must contain no `mod*` facts and no mod-made `BookReadState_*` flood. Other mods' state would sit in these nodes or in `questSystem`; not mapped.

### What a "vanilla, no DLC, no mods" template must look like on 5.x
1. All 34 top-level `BS` in this order, then `MANU` (identical on all 61 5.x saves): `CWitcherGameResource, saveInfo, InitialBuildIDBlock, facts, questSystem, community, attitudes, parentGroups, actorAttitudes, CJournalManager, CWitcherJournalManager, CR4GwintManager, StrayActors, CModHandlerSystem, SoundClues, ScentClues, FocusModeVisibility, CCommonMapManager, loot_manager, CR4TutorialSystem, SoundSystem, NewGamePlus, worldInfo, AdditionalSaveInfo, AdditionalContent, ContainerManagerSaveInfo, TimeScale, TickManager, universe, worldState, gameProp, timeManger, idTagManager, envManager`.
2. `CModHandlerSystem/ModNameSaveData`: `IsModdedSave=false`, `Size=0`. `NewGamePlus`: `Enable=false`, `active=false`. `ContentTaintedFlags=12`. json `numMods` 0.
3. `universe` has no `dlc\...` world and `worldInfo/world` is not under `dlc\`; `facts` has no `q7xx` facts and at most the two base `q6xx` defaults; no `mod*` facts.
4. DLC "installed" state of the template: `AdditionalContent` lists the 10 packs and `facts` has the five `*_installed` facts. This is what every 5.x save from a full install looks like (lukwraith's first save has both). A save from a machine without the packs would have `count 0` and no `*_installed` facts (2018 evidence, old format, not seen on 5.x).
5. A template taken in Kaer Morhen / White Orchard / Vizima (act 1) satisfies 2-3 and has no DLC world or quest markers. Whether the game refuses a save whose `AdditionalContent` names a pack that is not installed is UNTESTED.

### Unresolved and the test that would settle each
- Does the game react when `AdditionalContent` names a pack the machine lacks? Test (safe, one copy): add a fake `dlc_999_001` name (count 11) to a 5.01 copy via the typed tree and load it; compare with the unmodified repack. Real DLC absence can only be tested on a machine without the packs.
- Is `ContentTaintedFlags` bit 6 "mods"? Test: save once from a modded session on a vanilla playthrough and see 12 -> 76; or check a no-mod re-save of a hosam save.
- Meaning of `contentN`: edit a Novigrad save to `[0,1]` only (or a Kaer Morhen save to more) and load; or compare saves at the same spot with different DLC state.
- Third header int and the two zero u32: section 3.
- `a` in the file name: collect many saves in one session and correlate with the tracked quest hash / map pin.
