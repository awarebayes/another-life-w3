# Findings: character level, XP, skill points, vitality, crowns, items (start kit)

Date 2026-11-05. Evidence: 281 real saves (C:\w3-fixtures, C:\w3-extract incl. the Nexus packs; 215 read cleanly, 131 of them at
level <= 50), the game's scripts (`game\gameplay\leveling\levelManager.ws`, `ability\PlayerAbilityManager.ws`, `player\playerWitcher.ws`)
and item/ability definitions in the install bundles (`gameplay\abilities\geralt_levelups.xml`, `geralt_stats.xml`, `gameplay\items\*.xml`).
The third-party editor's README was read for behaviour only. Nothing is proven in game: "proven" below means proven offline
against saves and scripts. Code: `crates/w3save/src/character.rs`, `crates/w3al-core/src/start_kit.rs`.

## 1. Where the player lives [proven]

`universe > world > layerStorage > entityData` whose `idTag` = `01 00*12 01 00 00 00`; exactly one per save (281 of 281). Its `SS` body is
`SXAP` + a record stream, 281 of 281 re-serialize byte for byte after parsing:

* `BLCK u16 name, u32 size, records`; `AVAL|PORP u16 name, u16 type, u32 size, value` (same grammar as `name-remap.md`).
* Top level: `BLCK Entity {AVAL nP, PORP ...}`, `BLCK stats`, `timers`, `ai`, `at`, `tags`, `AVAL a`, then a **raw tail** = native inventory data.
* `PORP x:handle:Class` = object: `00 01 u32 id`, `u16 class`, `u8 0`, properties `u16 name, u16 type, u32 size (counting itself), value`, `u16 0`.
* Struct arrays: `u32 count`, per struct `00`, fields, `u16 0`. **The game leaves out zero-valued fields** (`SSpendablePoints.free = 0`).

## 2. Level, XP, skill points [proven offline]

`levelManager:handle:W3LevelManager` (`levelDefinitions`, `level`, `points[ESkillPoint, EExperiencePoint] {free, used}`, `lastCustomLevel`).
* `levelDefinitions` is the full XP table inside the save (51 rows, `lastCustomLevel` 50): level k = row k; row 0 is the "infinite levels" row. It equals
  `geralt_levelups.xml`. We read the table from the save, nothing is hard-coded.
* Invariant (`GainLevel`): experience pool `used` = `requiredTotalExp(level)`, `free` = leftover XP. Holds on 131 of 131 saves with level <= 50.
* Skill points: `free + used` = table grant (`addedSkillPoints`, levels 2..=level) + points from other sources (quests, books); the extra stays on a level change.
* Ability list `stats > characterStats > AVAL value (array of CName)` holds one `Lvl<N>` per level 2..=level (131 of 131, set equality). `AddPoints` does `AddAbility(GetLevelupAbility(level))`.
* Skill slots unlock themselves on load (`PlayerAbilityManager.PostInit` recomputes `unlocked` from `free+used`): not written.

## 3. Stored vitality / toxicity [proven offline / hypothesis]

`abilityManager > statPoints` = `SBaseStat {current, max, type}`; the element without a `type` field is vitality (hypothesis: first enum value = default = omitted).
* Vitality max = 3500 + sum of the `Lvl<N>` vitality bonuses (100 for levels 2-5, 80 for 6-10, 60 for 11-20, 50 for 21-29, 30 for 30-39, 25 for 40+; `geralt_stats.xml`) on 122 of 131 saves; the other 9 carry extra vitality from gear/skills.
* On 6 real level-ups (same playthrough) our edit reproduces the ability list (6/6), the skill-point change (6/6) and the stored-max change (5/6; the miss changed gear).
* `set_level` adds the same delta to stored `current` and `max`. [hypothesis] the game does not recompute the stored stat from abilities on load.
* Toxicity max is 100 on every save up to level 68 (only mutagen skills raise it); the editor README's "toxicity moves with level" is **not** seen here: we do not touch it.

## 4. Crowns and item records [partly proven]

Tail layout (5.x saves, `SXAP 67 29 164`): `00 00 | 01 a2 15 35 b6 | u16 A | u16 B | u16 count`, then item records, then a constant 6-byte trailer `01 00 00 00 00 00`.
`count` equals the number of records (found by the shape below) on 157 + 58 saves; the other 66 have record shapes not understood (3 dye names, extra `k7` bytes) and are refused.
Record (understood part): `u16 name, u16 hash, u16 flags, 01 00 00 00 00 00 00, u16 dye1, u16 dye2 (Dye Default / dye_default), u16 qty, f32 durability (-1.0 = none), u8 nExtras, nExtras x (u16 name, u32 value, 02), u16 uid, u8 0, u8 nAbilities, nAbilities x u16 ability name, u16 slot [, u16 0 if slot != 0]`.
* `uid` equals the ids in `itemSlots` (equipped items) [proven on one save]; `hash` is constant per item instance over a playthrough (per-instance seed [hypothesis]).
* Crowns = quantity of the `Crowns` record, edited as `u16`. Values above 65535 are refused: the highest u16 seen is 65477 and 20 records use another layout, so the wider field is NOT proven.
* Potions carry `ammo_current` / `is_initialized` extras; weapons/armour carry durability, `ItemDurabilityModified`, an ability list (`autogen_steel_dmg`, ...) and sometimes an enchantment block (`z = 1`, not decoded).
* **Not done (Error::Unverified):** creating a record for an item the character does not carry. Needs the real meaning of `hash`, `flags`, the list header `B` (is not "max uid": 70 vs uids up to 905 on one save) and the ability lists. So **no weapons or armour are added**.

## 5. Start kit behaviour

`level`: `set_level` (section 2-3). `crowns`: section 4. `gear = "recommended"`: raise stacks the template already has (Bread, Bodkin Bolt, repair kits; names from the item XML) to the band's amount; missing ones are reported and skipped. After export the file is re-read and compared.

## Open hypotheses

1. Game accepts the edited level data, `Lvl<N>` ability appends and the vitality bump (no in-game gate yet).
2. Stored max/current vitality not recomputed on load.
3. u16 crowns quantity; `statPoints[0]` = vitality.
4. Quantity-only edits keep records valid (`hash`, uid untouched).

## 6. Adding new items (second pass, 2026-11-05)

Record layout (5.01, `character::parse_records` tiles the whole list to the trailer on 140 of 163 `SXAP 67` saves):
`u16 name | u16 hash | u16 flags | 01 00 00 00 00 00 | u8 slots | u16 dye1 | u16 dye2 | u16 qty | f32 durability | u8 nExtras | nExtras x (u16 name, u32 value, 02) | u16 uid | u8 z(=0) | u8 nAbilities | nAbilities x u16 | u16 slot [| u16 0 if slot != 0]`, then the 6-byte trailer.

Decoded [proven offline unless marked]:
* **slots** (the byte after the six fixed ones) = `enhancement_slots` of the item definition (Dol Blathanna longsword / Armor, Silver Vixen: 1; the set's boots/gloves/pants: 0).
* **flags**: 0x000f potions/oils/bombs/ingredients/bolts; 0x0008 sword and 0x000a armour piece as quest rewards; +0x100 looted, +0x20 equipped (then `slot` names `steelsword`, `armor`, `boots`, ...). [meaning of the bits inferred from the groups, not from scripts]
* **durability** = `initial_durability` (100) for a fresh piece, -1.0 for stacks and consumables.
* **uid** is ascending through the list; **header field B = last uid handed out** (B == max uid on 132 of 140, B > max on 8 where items were removed, never below). A new record takes B + 1, B moves with it.
* **Charged items** carry extras `ammo_current` and `is_initialized`: tier-1 potions 3, oils 20, bombs 2 (92 of 92 fresh records).
* **Autogen gear** (`Autogen` tag, looted weapons/armour): the ability list is `autogen_*_base` plus one `autogen_*_dmg|armor` per level from `inventoryComponent.ws` (`AddItemCraftedAbility` loop, level from quality and player level with random jitter); the stored ORDER varies between saves and the NG+ variant uses `autogen_fixed_*`. Not reproduced: autogen gear is refused. The kit uses only fixed-stat pieces (no ability list).
* **hash** is constant per item instance and differs between instances; derived (FNV of name and uid), unproven that any value is accepted.
* Not decoded: `z = 1` records (enchantment / rune block, e.g. 52586 `Nilfgaardian sword 1 schematic`), pack_501 records with a third name before the quantity; saves containing them are refused for `add_item`.

Acceptance (test `built_records_equal_the_records_the_game_wrote`, with the Nexus packs): the record built for `Dol Blathanna longsword`, `Silver Vixen`, `Dol Blathanna Armor`, `Dol Blathanna Boots` (4 of 4 each), `Cat 1`, `Beast Oil 1`, `Grapeshot 1` (92 of 92) and `Blunt Bolt` (18 of 18) is byte for byte the record the game wrote, given that record's uid, hash and quantity. No pair of vanilla saves of one playthrough with a looted item was found (only NG+ re-saves), so uid allocation rests on the B statistics above, not on a before/after pair.

Kit per level band (`start_kit.rs`, names and slots from the item XML; the piece level is the third-party catalog's lead): 1-10 Dol Blathanna longsword + Silver Vixen + Dol Blathanna set (L7); 11-17 Gryphon School swords + Gryphon set (L11); 18-25 (L18); 26-33 (L26); 34+ (L34); plus Swallow 1, Cat 1, Beast Oil 1, Grapeshot 1 and bread / bolts / repair kits (raised when carried, created otherwise). Items already carried are left alone. Hypotheses: flags 0x0008 / 0x000a for crafted-school pieces, derived hash, the game accepting the appended records.

## 7. The remaining record variants (third pass, 2026-11-05)

`parse_records(tail, version)` now tiles the item list of **281 of 281** fixtures (163 `SXAP 67`, 1 `66`, 117 `64`), header + records + 6-byte trailer, byte-exact. What was decoded [proven offline by that tiling]:

* **`flags & 0x2000`: the quantity is a `u32`.** Every Crowns record above 65535 carries it (pack_501 purses 114114 .. 231474). So the "third name before the quantity" of the pack_501 saves was the high half of a big purse, not a third dye. `set_crowns` widens a narrow record (turns the flag on, inserts two bytes) when asked for more than 65535 [hypothesis: the game reads a record we widened the way it reads its own].
* **`flags & 0x4000`**: an extra `u32` before the quantity (one modded NG+ save, 2 records).
* **`flags & 0x8000`** (dyed pieces, the dye names differ from `Dye Default`): after the equipped slot comes `u8 n, n x u16`.
* **Enhancement entries**: after `uid` the byte `z` is the number of 9-byte entries (`u16 a, u16 b, 00, 00 01, u16 c`), then `nAbilities`. 160 records in the user's Novigrad save and the Nexus packs. Opaque, copied verbatim.
* **Slot**: the equipped-slot name exists only when `flags & 0x20`. `SXAP 66/67` records end with an extra `u16 0`; `SXAP 64` records do not.
* The first two tail bytes are `00 00` or `6a 28` (4 saves): not a constant; uids are not always ascending (NG+ saves): `add_item` takes `B + 1` where `B` >= every uid.
* Levels above the XP table (the templates are NG+ characters at level 68-86): experience past the table costs row 0's `requiredExp` per level, skill points row 0's `addedSkillPoints`; no `Lvl<N>` ability exists past 50. `set_level` can lower such a character; spent skill points above what the target level grants are kept (free = 0) [hypothesis: the game does not check them against the level].

Per template (C:\w3-fixtures\pack_501, level 30 + 5000 crowns + recommended kit, written file re-read, VERIFIED): 10028e (`SXAP 64`, 641 items) 9 items added; 100840 (839) 10; 105a8e (885) 12; 4dcef (871) 12; 4e4ce (903) 11; 54142 (901) 9; 54acc (693) 10; 86355 (893) 12; 8c0ab (668) 11; user's 52586 (339) 12. Items already carried (repair kits, potions, bread) are not added again.
