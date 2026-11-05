# NM name remapping for graph transplants

Date 2026-11-05. Builds on `quest-graph-design.md` (sections 3 and 5). Code: `crates/w3save/src/graph.rs` (`NameMode`,
`transplant_with`, `remap_node`, `remap_records`, `resolved_fingerprint`), tests: unit tests in `graph.rs`, fixture tests at the end of
`crates/w3save/tests/graph_fixtures.rs`. Labels: **[proven]** = checked by a test on real saves, **[hypothesis]** = needs the game.
Fixtures: `W3AL_FIXTURES=C:\w3-extract` (101 saves, 5 playthrough folders); the decode tests were also run on `C:\w3-fixtures` (25 5.x saves).

## 1. Problem

NM indices are per save. `NameMode::Prefix` (the old `transplant`) needs the template's NM table to be a prefix of the donor's and
appends the donor's extra names. Two donors played from one template append different names at the same indices, so the second delta
cannot go on, and donors from other playthroughs are refused. `NameMode::Remap` translates every index in a copied subtree to the
template's index for the same name (interning missing names), including the indices hidden in opaque values.

## 2. Inventory: opaque values in `questSystem` and `community` [proven]

Over 101 saves (`every_opaque_value_in_the_graph_blocks_decodes` prints it):

| Block | Type | Count | Where |
| --- | --- | --- | --- |
| community | `EngineTime` | 43,684 | `spawnDelay` |
| community | `SActionPointId` | 24,598 | `currentAPComponentID`, `lastAPComponentID` |
| community | `IdTag` | 12,299 | `idTag` |
| community | `Vector` | 8,800 | `position` |
| questSystem | `array:2,0,SQuestThreadSuspensionData` | 53,731 | `suspendedScopesData` (almost all `00000000`) |
| questSystem | `array:2,0,CName` | 845 | `questBlock > ActivatedInputs` |
| questSystem | `GameTime` | 32 | `conditions > timeRemaining` |
| questSystem | `IdTag` | 2 | `treeState > entryGUID` |
| questSystem | `SS` section | 2 | `questBlock > treeState > SS` |

No other opaque type, no `Enum` value and no raw node outside these values occurs in the two blocks. Decoded `CName` values
(`inputName`, `tag`, `activePhase`, `appearance`, `actionCategoryName`) were already typed and are remapped like names.

## 3. Layouts and how each is handled

All of these are decoded; every byte of every value is accounted for, and a value that does not tile exactly is refused.

* **Structs (tree form)** [proven]: `00`, then per field `u16 name, u16 type` and the value as a nested untagged table entry, then
  `u16 0`. `Vector` = `00 {X:Float}{Y:Float}{Z:Float}{W:Float} 0000` (35 bytes, the old fixed width), `GameTime` = `00 {m_seconds:Int32} 0000`
  (11), `EngineTime` = `00 0000` (3, no fields, all 43,684), `SActionPointId` = `00 {component:CGUID}{entity:CGUID} 0000` or `00 0000`.
  Remapped: field name and type. Field values seen: `Float`, `Int32`, `CGUID`, and `scopeData` below; anything else is refused. The
  leading `00` is 00 on every struct; its meaning is unknown and it is required to be 0.
* **`array:2,0,CName`** [proven]: `u32 count` + `count x u16` name index. All 1,058 elements in 101 saves name an input pin (`In`,
  `In 1` .. `In 23`). 0 would be the empty name and stays 0 (never seen).
* **`array:2,0,SQuestThreadSuspensionData`** [proven]: `u32 count` + count structs `00 {scopeBlockGUID:CGUID}{scopeData:array:154,0,Uint8}
  0000` (the type is spelled `array:154,..` or `array:159,..` depending on the save). `scopeData` = `u32 length` + a **record stream**:
  the suspended `questThread` saved with the same names and types as the tree:
  `BLCK u16 name, u32 size, records` / `AVAL|PORP u16 name, u16 type, u32 size, value`. Record values seen: `Uint32`, `Int32`, `Uint16`,
  `Bool`, `CGUID`, `Float`, `CName` (remapped), `array:2,0,CName` (remapped), an empty `suspendedScopesData` (`00000000`; never non-empty
  inside a stream, a non-empty one is refused) and `timeRemaining:GameTime` as an inline struct `00 {u16 name, u16 type, u32 size
  (counting itself), value} 0000` (15 bytes; 1 value in C:\w3-extract, 5 in C:\w3-fixtures).
* **`treeState > SS`** [proven]: `SXAP` + three ints + the same record stream (`BLCK ai {AVAL savedNodesCount:Uint16, AVAL hash:Uint16,
  AVAL ArrivedAtPath:Bool}`). Remapped like `scopeData`.
* **`IdTag`** (17 bytes) [proven: no name index]: `01` + 16 bytes; bytes 9..13 are 0 on all 23,072 values (both fixture sets); the only
  small integer (the last u32) is larger than the save's NM table in 18 values (3 saves) and points at unrelated names elsewhere, so it is
  not a name index. Copied verbatim; a first byte other than 0/1 is refused.
* **Refused** (`Error::Unverified`): `Enum` values (EJournalStatus & co. are still a hypothesis as name indices), any other opaque type,
  any struct field / record value of another type, `MANU`, raw nodes outside a decoded value, `SS` sections that are not a record stream.

Atvaark's W3SavegameEditor (MIT) was not needed; the layouts come from the bytes. Nothing was ported.

## 4. Evidence

* **Every value decodes** (`every_opaque_value_in_the_graph_blocks_decodes`): for both blocks of 101 + 25 saves, the identity remap
  gives the same bytes (layouts tile every value), every decoded index position holds a valid NM index (1..=table length), and remapping
  through a scrambled table (`i -> n + 1 - i`) and back gives the same bytes (the remapped block decodes on its own).
* **No index position is missed** (`translated_values_equal_the_games_own_bytes_across_name_tables`): where two saves with different NM
  tables hold the same value (equal by `resolved_fingerprint`), translating one save's bytes into the other's table must give exactly the
  bytes the GAME wrote in the other save; a missed position would keep the wrong number. C:\w3-extract: 39 save pairs (first/last save of
  each folder against the others, plus lukwraith 06/07), 27,978 equal values checked, 98 of them with differing bytes (`SActionPointId` 10,
  `Vector` 10, `ActivatedInputs` 66, `suspendedScopesData` 12), all equal to the game's bytes after translation. C:\w3-fixtures: 80 pairs,
  52,361 values, 116 translated (`ActivatedInputs` 92, `suspendedScopesData` 24). `GameTime` timers and `treeState` sections never hold
  equal content across two tables, so for them only the tiling evidence exists.
* **Consecutive pairs in remap mode** (`consecutive_graph_diffs_replay_in_remap_mode`): 88 lukwraith pairs; on the 86 whose names are a
  prefix the result's `questSystem` + `community` are byte-identical to B's; 06 -> 07 (twice; prefix mode refuses it) now replays: 153 ops,
  11 names interned, both blocks equal B's by `resolved_fingerprint` AND by `graph::fingerprint`; the count/scene rules hold on every result.
* **Two donors on one template** (`two_donors_from_two_branches_onto_one_template`): lukwraith 06 and 07 are both played from 05 (05's
  names are a prefix of both, neither of the other; index 2432 is a different name in each). Delta 1 = all of 05 -> 06 (17 stub changes in
  `community`); delta 2 = 05 -> 07's `questSystem` (33 scopes) plus the five stubs 06 left alone (a#10, a#16..a#19). 06 then 07: 07's step
  interns 2 names (`__q002_nilfgaard_commander`, `__q002_peasant`, used in stubs a#18/a#19) at other indices than in 07; prefix mode
  refuses this step. Every transplanted scope equals its donor's, `communities` / `numStubs` / a#22 stay 05's, 999 count/scene checks
  hold. 07 then 06 gives the same blocks. 07's removal of the `q001_fistfight_thugs` spawn set and its stubs a#20..a#22 is left out
  (it would drop 06's a#20/a#21).
* **Other playthroughs as donors** (`whole_graph_blocks_from_other_playthroughs`): the whole `questSystem` + `community` of 31 saves
  (11 from three other playthroughs: hosamnasr 1-9, imz, whiteorchard; 19..1,125 names interned) onto lukwraith 21: both blocks equal the
  donor's, `facts` / `CJournalManager` / `universe` stay byte-identical, the rules hold. Offline only.
* Unit tests on hand-written values (`opaque_values_remap_to_the_same_names`, `undecoded_values_are_refused`): each value is built for two
  NM tables by one builder; remapping one gives exactly the other and back.

## 5. API

* `graph::transplant(template, donor, ops)`: unchanged (`NameMode::Prefix`), byte-exact as before; its guard hashes (`fingerprint`) unchanged.
* `graph::transplant_with(template, donor, ops, NameMode::Remap)`: no prefix requirement. If the template's names happen to be a prefix of
  the donor's, the donor's extra names are appended first (as Prefix), so the translation is the identity and the bytes equal Prefix mode;
  the walk still runs and still refuses undecoded values. Otherwise missing names are interned in order of first use. The check after
  writing compares replaced/inserted subtrees by `resolved_fingerprint`.
* `GraftReport::names_added` (names appended, both modes), `GraftReport::names_remapped` (donor indices at a different index in the result).
* `graph::remap_node(names, node, f)`, `graph::remap_records(names, bytes, f)`: the walker, usable on its own.
* `graph::resolved_fingerprint(doc, node)`: index-free fingerprint (the subtree remapped onto local indices in order of first use, plus those
  names); equal for a remapped copy and its donor. `fingerprint` stays as it is because TOML guards hash it.

## 6. Still hypotheses / open

1. **[hypothesis]** The game accepts names we appended and runs a remapped subtree (no in-game test of remap mode yet; gate G covered a
   prefix-mode transplant of one quest). A natural gate: the two-donor result above, or mq0005 from the user's own J1 completion onto 21.
2. **[hypothesis]** Mixed type spellings: a donor's `array:154,0,Uint8` interned into a template that uses `array:159,0,Uint8`. The game itself
   carries both spellings in 10 of the 5.01 saves, which suggests it reads either, but that is not shown for a value we moved.
3. A remap is only as good as the scope selection: the two-donor test splits by stub; mixing quest state across playthroughs (whole blocks
   above) is a format proof, not a claim that the game state is coherent.
4. `Enum` values, other struct types (`EulerAngles`, `Vector2`, ...) and other blocks (`universe`, `CJournalManager`) are refused by remap
   mode until someone decodes them; `CJournalManager` edits keep using `journal::apply`.
5. Inline structs in record streams and `treeState` sections rest on tiling evidence only (6 and 5 values); no cross-table equal value exists.
6. The struct lead byte `00` and `IdTag`'s 16 bytes are uninterpreted (copied).
