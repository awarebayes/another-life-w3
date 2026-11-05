# Findings: loaded world, player position, spawn (plan unknown 4)

Date 2026-11-04. Read-only research on copies of real saves. Throwaway tooling (not in the repo):
`<scratch>/position/src/main.rs (a throwaway scratch tool, not in the repo)`
(a cargo project with a path dependency on `crates/w3save`, own target dir). Subcommands: tree dump, `player`, `scan`, `worlds`,
`spawn`, `blck`, `mk` (test-save builder), `chk`. Everything below marked **(inference)** is not proven in game.

Saves used: lukwraith set (45 saves, 5.01: Kaer Morhen -> White Orchard -> Vizima palace -> White Orchard), the user's Novigrad save
`ManualSave_52586_7ea4c400_1030937` (5.01), hosamnasr (9, 5.01, mods), imz (1, 5.01), WestAard (1, saveVersion 66) and the 17
`C:\w3-fixtures` saves (16 are older game versions; the layout below is identical in them).

## 1. Where the loaded world is stored

| What | Node path | Type / value |
| --- | --- | --- |
| Current world (authoritative) | `worldInfo` > `world` | `VL`, `String` (packed). Full level path |
| Load-screen picture/map label | `AdditionalSaveInfo` > `LoadingScreenName` | `VL`, `CName` = `map_location_<short name>` |
| World the player entity lives in | `universe` > `world` (one `BS` per world) > `layerStorage` > `entityData` whose `idTag` = `01 00..00 01 00 00 00` | the player entity (56-57 KB early game) |
| Visited flags | `CCommonMapManager` > `CachedWorldDataMap` > `CachedWorldDataPath` / `CachedWorldVisited` | one pair per world, `Bool` |

World strings seen (`worldInfo` > `world`):

| String | Where | Seen in |
| --- | --- | --- |
| `levels\kaer_morhen\kaer_morhen.w2w` | Kaer Morhen (prologue and later visit) | lukwraith 01, 02 |
| `levels\prolog_village\prolog_village.w2w` | White Orchard | lukwraith 03-37, 43-45; WestAard |
| `levels\wyzima_castle\wyzima_castle.w2w` | Vizima palace (Imperial Audience) | lukwraith 38-42; 4 older saves |
| `levels\novigrad\novigrad.w2w` | Velen, Novigrad, No Man's Land (one world) | user's save + 13 others |
| `levels\skellige\skellige.w2w` | Skellige | `6b4a7_7e12cc00_4e232cb` |
| `dlc\bob\data\levels\bob\bob.w2w` | Toussaint | 6 saves |
| also in `CachedWorldDataMap` only | `levels\prolog_village_winter\prolog_village.w2w`, `levels\island_of_mist\island_of_mist.w2w`, `levels\the_spiral\spiral.w2w`, `dummy.w2w` | never current in our set |

Evidence the string is what the game uses: it flips exactly at the story transitions. lukwraith 02 = kaer_morhen, 03 = prolog_village
(guide: "on the way to White Orchard"), 37 = prolog_village, 38 = wyzima_castle (Imperial Audience start), 42 = wyzima_castle, 43 =
prolog_village (back). `LoadingScreenName` changes in lockstep on all 45.

`universe` layout (5.01): `BS universe` { `BS Player`, `VL count`(u32 = number of world blocks), N x `BS world` }. A `world` block is
{ `BS visibility`, `VL name` (the same level string), `SS` (SXAP + BLCK stream, `worldLayerGroups` / layer data), `BS layerStorage` }.
`layerStorage` = { `VL numEntities`, N x `BS entityData` } and `numEntities` equals the number of `entityData` children on all
saves checked. Block order in the file is first-visit order, not "current first".

World blocks are created lazily and persist. Kaer Morhen-only save (lukwraith 01): `universe.count` = 2 (kaer_morhen full, novigrad
stub). After the transition (03): count = 3 (prolog_village added, 275 KB). The stub the game itself writes for an unvisited world is:
`SS` of 6 bytes (`53 53 00 00 00 00`, no SXAP), `layerStorage` of 14 bytes (`numEntities` = 0), a small `visibility` (192 bytes on a
fresh game). Stubs for novigrad (from the first save), skellige (from lukwraith 42) and `prolog_village_winter` exist in
early saves. Late-game saves keep a full block for every visited world (novigrad SS 2.1-2.6 MB, skellige 1.7-1.9 MB, bob 1.4-1.9 MB).

When the world changes, the player entity moves: in 02 -> 03 the kaer_morhen block ends with `numEntities = 0` and the prolog_village
block holds the player; 37 -> 38 and 42 -> 43 likewise (the old world keeps its other entities: wyzima_castle keeps 18).
`playerEntity=true` is in exactly one world block on every save (the current one).

## 2. Where the player position and rotation are stored

Single place: `universe` > `Player` (a `BS` with 6 children):

| Child | Type | Content |
| --- | --- | --- |
| `id` | `IdTag` (17 B) | `01 00 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00` on every save |
| `position` | `Vector` (35 B) | see below |
| `rotation` | `EulerAngles` (27 B) | see below |
| `template` | `String` | `gameplay\templates\characters\player\player.w2ent` |
| `count` | `Uint32` | 1 |
| `idTag` | `IdTag` | = the id of the `player_horse_manager` entity (a link, not the player) |

`Vector` value (35 bytes), proven on all 5.01 families and on the old-format saves:

```
00                                    1 byte, always 0
{ u16 name_X  u16 type_Float  f32 x }   name_X = NM index of "X" (960 in the lukwraith family; differs per save family)
{ u16 name_Y  u16 type_Float  f32 y }   consecutive NM indices X,Y,Z,W
{ u16 name_Z  u16 type_Float  f32 z }
{ u16 name_W  u16 type_Float  f32 w }   w = 1.0 (0x3f800000)
00 00                                 2 bytes
```

Value-relative offsets: x at 5, y at 13, z at 21, w at 29. `EulerAngles` (27 B) = `00`, then Pitch, Yaw, Roll members in the same
(name, type, f32) form (names 1588/1589/1590 = `Pitch`/`Yaw`/`Roll`), then `00 00`; f32 at 5, 13, 21. Yaw is the **second** float,
in degrees, 0..360. Pitch and roll are 0.0 on every save. The four (three) floats are separate raw table entries (4 bytes each, untagged,
children of the `position` / `rotation` node), so `Document` already exposes them as `Kind::Raw` children with one `Part::Bytes`.
I confirmed the NM names by decoding: index 960..963 = `X Y Z W`, 60 = `Float`, 1588..1590 = `Pitch Yaw Roll` on a lukwraith save.

Evidence that these floats are the player's position (all from the saves, not scripts):
- Same spot, same coordinates: lukwraith 30 "at the Nilfgaardian garrison" = (-325.455, 386.682, 15.114); lukwraith 36 "at the garrison, talk
  to the Captain" = (-324.270, 384.849, 15.112). 23 "Precious Cargo start (Geermer)" = (-299.477, 202.266, 2.250); 24 "confront Geermer" = (-298.186, 203.097, 2.400).
  04 (tavern area) = (55.10, 10.87, 2.353); 43 and 37 (same area) = (50.7, 13.6, 2.37) and (55.9, 10.6, 2.353).
- Nearby pair 30 -> 31 (garrison tower, "Beast of White Orchard" start): (-325.5, 386.7) -> (-337.4, 347.4), z 15.11 -> 14.85. 33 -> 34 moves (-32, -171) -> (58, 0.6).
- Far pair: Kaer Morhen 01 (53.2, -5.3, 196.6) vs White Orchard 03 (515.2, 81.4, 32.1). 03 is right at the q001 ghoul start scene
  (spawned ghouls at (505..518, 75..88, 31..33)).
- Different playthroughs saved at a scripted arrival give identical values: three old-format Vizima saves (`1004de_c1c1f2`, `4dceb_c2d64a`,
  `55607_c32db0`) all at (11.719066, -18.981781, 10.132781), yaw 10.4257. Quest scripts, not the player, chose that spot.
- Coordinates are world-local: Kaer Morhen and White Orchard are both in the +-600 range, so a position alone does not identify the world.
  Novigrad world spans about (-2150..2500); Skellige sample (-2146, -306); Toussaint (-402, -800), (248, -2096).
- Z is the ground height at that spot (0..60), so a hand-typed z is a risk (see 5).

Duplicates: a byte search of the whole decoded payload for each of the three f32s of `Player/position` finds exactly **one** hit each (the
`Vector` itself) on 4 saves (lukwraith 01, 30, 38, user's Novigrad). So the position is not repeated as a float anywhere else (no checkpoint or
teleport copy, and not inside the 57 KB player entity blob). Other `position` / `rotation` vars: `community` > `stubs` (461 NPC spawn stubs, rotation is a plain Float) and
`CCommonMapManager` cached map pins; neither is the player. Entities with `hasSpawnData` carry `p` (Vector), `r` (EulerAngles), `s` (Vector scale) in the
same layout (`player_horse.w2ent`, wolves, bandits, boats). The horse is NOT placed relative to the player: e.g. lukwraith 30 player (-325, 387) vs
`player_horse` (-257.6, 67.0); the user's save: player (1707.9, 1068.2) vs `player_horse` (1595.8, 854.9).

Camera: the save has no camera transform. NM contains `autoCameraCenterToggle` (a Bool inside the player entity), `TutorialCamera`,
`cameraEffectName`; nothing positional. **(inference)** the camera is derived from the player rotation at load.

## 3. What decides where the player appears on load

Observed (facts): the only player-position data is `universe/Player/position` + `rotation`; the level is `worldInfo/world` (and the world
block that contains the player entity). There is no "last safe position", checkpoint or spawn-point reference in the 5.01 NM names
(searched `spawn`, `teleport`, `checkpoint`, `waypoint`, `safe`, `fast`; hits are monster spawners, quest facts, the fast-travel manager).
**(inference)** The game restores the player at `Player/position` in the world named by `worldInfo/world`; any quest-driven re-placement
happens in scripts after load. Needs the in-game test below.

Mount state: the player entity's `ai` block has `wasMountedWhenSaving`, `vehicleSlot_save`, `wasMountedOnBoatWhenSaving` (all false in
every save I checked: lukwraith 30, 45, user's). A template saved on Roach or a boat would carry this separately from the position
(inference: teleporting would not dismount).

## 4. Edits

### 4a. Teleport inside the same world (minimal)

Overwrite 3 f32 (x, y, z) in the `position` node and 1 f32 (yaw) in the `rotation` node:
`universe` > `Player` > `position` : Vector -> raw children 0, 1, 2; `rotation` : EulerAngles -> raw child 1 (yaw).
Equal size, so no table/RB/footer change and `rebase` is a no-op. In bytes: value offsets 5, 13, 21 and (rotation) 13. Keep the member
headers (name/type u16s) and the `w = 1.0`. API shape for `w3save::position`: `get(&Document) -> {world: String, x, y, z, yaw}`;
`set` writes the four floats; reject non-finite values.

I built this edit with the tree API (`Part::Bytes` replace on the raw children, `rebase`, re-parse, table equals the tree) and
wrote a test save (below). It re-parses, the table equals the tree, and `player` reads back the new values. NOT loaded in game.

### 4b. Changing the world

Needed, in this order (inference from how the game writes the transition, 02 -> 03, 37 -> 38, 42 -> 43):
1. `worldInfo` > `world` string = target level path (length changes, `rebase` handles the growth).
2. `AdditionalSaveInfo` > `LoadingScreenName` CName = `map_location_<short>` if that name is in the save's NM (cosmetic;
   if the name is missing the NM table would have to grow; not needed to try).
3. Move the player `entityData` (idTag `01 00..00 01 00 00 00`) from the old world's `layerStorage` to the target world's `layerStorage`, and fix
   both `numEntities`. The node keeps its table index; only the parent changes, so the table entry count is unchanged and `rebase` copes.
4. Set `Player/position` + `rotation` to a valid spot in the target world (coordinates are per world).
5. The target world needs a `universe` > `world` block. If the template has one (every visited world keeps its block, including stubs), use
   it. If not, a stub could be fabricated (`SS` `53 53 00 00 00 00`, `layerStorage` with `numEntities` 0, a `visibility` BS, `name`), and
   `universe.count` + 1. The game itself writes such stubs for worlds the player has not entered (novigrad from save 1), so a stub is a
   valid on-disk state (fact).
6. `CCommonMapManager` visited flag (probably cosmetic).

What changing the world does NOT bring (the data is persisted per world, not derived from facts):
- layer/world state: `world` > `SS` (worldLayerGroups, 51 KB in KM, 300 KB in WO, 2.1 MB in Novigrad), `layerStorage` entities (the NPCs
  and monsters currently spawned), and `visibility` (per-world lists of u64 hashes that grow with progress: novigrad 192 -> 1508 -> 15508 ->
  36648 bytes over the course of the game; **inference**: quest-driven layer visibility). A stub world therefore shows its start state
  whatever the quest facts say. This is why a template from the right world and the right point in the story is needed for anything beyond the teleport.
- `questSystem`, `community`, `CJournalManager` already live outside `universe` and are untouched by a world swap (they are what the other agents edit).

### 4c. Test saves built (not loaded in game)

All in `...\scratchpad\position\out\` with sidecars (.png/.json copied from the source, new stems in the `ManualSave_<a>_<b>_<c>` form):
- `ManualSave_8559a_7ea4d400_3d70001` (T1): lukwraith 04 (White Orchard, tavern) moved to the garrison spot of lukwraith 36 (-324.27, 384.85, 15.112, yaw 225.8). Same world. Expected: Geralt appears at the garrison.
- `ManualSave_8559a_7ea4e000_3bb9001` (T2a): lukwraith 37 (White Orchard) -> Kaer Morhen at the new-game spot (53.24, -5.35, 196.55, yaw 34.6): world string, loading screen name, player entityData moved into the existing kaer_morhen block, position. Expected: Geralt in Kaer Morhen with White Orchard-era quest facts.
- `ManualSave_52586_7ea4c400_1031001` (T2b): the user's Novigrad save -> White Orchard at (102.41, -6.25, 1.64, yaw 243.8) (lukwraith 45 spot): same steps into the existing prolog_village block (the user's save has it, 60 stale entities). Expected: Geralt in White Orchard with a late-game state.
All three: re-parsed, `Document::table()` equals the on-disk table, container write reads back.

## 5. Implications for the plan

- "Template per act": a **template per world is a hard requirement**, per act is a consequence. The world block (layers, spawned entities, visibility hashes)
  is part of the save and is not reconstructible from facts. **(inference)** One template can serve several acts only if they are in
  the same world (Velen + Novigrad + No Man's Land are one world `novigrad.w2w`; Skellige, Toussaint, Kaer Morhen, White Orchard, Vizima are separate).
  A world swap to a world the template has visited (4b, T2a/T2b) is worth an in-game try: if the game accepts it, the user's own save can
  be re-homed to any world they have visited, which makes "the player's own save as template" viable for most acts. An unvisited world (Skellige for a
  Novigrad-era save, Toussaint) needs a stub or a bundled block.
- Position edit is cheap (4 floats, no rebase). The risk is z: use coordinates harvested from real saves (the catalogue above, per world) rather
  than invented ones; the vizima palace arrival (11.719066, -18.981781, 10.132781, yaw 10.4257) is script-exact.
- Spawn design question (palace vs free-roam signpost): the position format supports either; the palace is a different world (`wyzima_castle`) that
  the game only enters via quest scripts, so a literal palace spawn needs a wyzima block (every save that passed the Imperial Audience has one; a stub would be needed otherwise).
- The horse does not follow the player (position independent); the game normally summons it. Unknown whether a teleport far from the horse is fine.

## 6. Unknowns that need an in-game test

1. T1: does a same-world position edit take effect (Geralt at the new spot, no fall through the ground, camera sane)? Also: does the quest/NPC state around the
   new spot look wrong (it should be the template's state)?
2. T2a / T2b: does the game accept a world swap done this way (loads, no CTD, correct world, sane NPCs)? If it loads the old world, the player entity move or the
   `numEntities` fix is not enough and another field decides.
3. Does the game need `universe` > world block order, or the target block's `layerStorage` to contain only entities of that world (T2b leaves 60 stale prologue entities + the player in it)?
4. Is `z` snapped to the ground on load, or must it be exact? (Try z + 5 and z - 0.5 on T1.)
5. Mounted/boat state at save time with a teleport (template saved on horseback).
6. Whether a fabricated stub world block is accepted (needs a template without the target block, e.g. the lukwraith 01 for White Orchard).
7. The `visibility` hash lists: which layers they represent and whether a late-game `visibility` on an early-game world matters.
8. Old format (`[64, 18..27, 163]`) uses the same layout for everything above, so this works for 4.x saves too; only 5.01 saves were considered for the product.
