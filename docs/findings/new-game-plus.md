# New Game+ in a 5.01 save, and turning it off

Date 2026-11-06. Offline research only (nothing launched). Question: every registered template comes from the Nexus 2394 pack
playthrough, which is a New Game+. A player who recovers a normal first playthrough gets an NG+ world (enemy scaling). Where does
the NG+ state live, what reads it, and what must change to make the save a normal game?

Evidence: the game's scripts (`C:\Program Files (x86)\Steam\steamapps\common\The Witcher 3\content\content0\scripts`, patch 5.x,
paths below are relative to `game\`), the quest graphs of the install (`crates/w3graph` bundle reader, strings of the extracted
`.w2phase` / `.w2scene` files), and 72 parseable saves under `C:\w3-fixtures` (`w3al facts`, `tree`, `character --items`,
`graph-diff`; a scratch item-record dumper over `w3save::character::parse_records`).

## 1. Where the state lives

| Where | NG+ save (pack 33 template, level 53) | Normal 5.01 save (`ManualSave_52586_7ea4c400_1030937`) | Evidence |
| --- | --- | --- | --- |
| fact `NewGamePlus` | 1, time 368.1 (the first seconds of the NG+ game) | absent | `w3al facts --diff` |
| fact `FinalNewGamePlusLevel` | 53 (50 on the older modded NG+ saves) | absent | same |
| top-level block `NewGamePlus` | `Enable=false`, `active=true` | `Enable=false`, `active=false` | `w3al tree --block NewGamePlus` |
| item records | 8 named `NGP ...`, 662 of 678 records carry `NGPItemAdjusted=1`, 18 carry 23 to 74 stacked `autogen_fixed_*` abilities | none | scratch dump |
| anything else | nothing: no `NGP` / `NewGame` name in `questSystem`, `community`, `universe`, `gameProp`, `CR4TutorialSystem`, the journal blocks (the only `NGP` hits in `CJournalManager` are the substring of `mq3041showingpresence`) | | `w3al tree --grep` over all 34 top-level blocks |

Across the 72 parseable fixtures: 43 NG+ saves have the flag fact, the level fact and `active = true` together (the Nexus pack, the
older modded NG+ saves); 29 saves have none of the three. No save has one without the others, except the Blood and Wine NG+-menu
save `ManualSave_11a7d0_7e10e000_5d08a45` (`Enable=true`, `active=false`, no fact): that is "NG+ unlocked, this run is not NG+"
(`Enable` is what `EnableNewGamePlus(bool)` writes, `quest_function.ws:6`, `r4Game.ws:625`, called by the `dlc9_new_game_plus` /
`bugfixing_phase` graphs with the message `message_new_game_plus_unlocked` when the game is finished).

Facts set by the NG+ start that are NOT NG+ state and stay: `simulate_import_ingame` and the `import_*` facts (the Witcher 2 import
choices the player simulated; they feed dialogue), `NGE_*` (next-gen edition), `ClearingPotionPassiveBonusFix` (a one-time fix,
`player/playerWitcher.ws:751`), `bugfix_*` / `Patch_*` (time 368 vs ~59 on a normal new game, only the clock differs).

## 2. What the scripts do with it

Everything is gated on the fact (`FactsQuerySum("NewGamePlus") > 0`). The level fact only feeds numbers inside those gates.

Enemy scaling (the symptom):

* `npc\npc.ws:613-623`: on spawn, every non-animal NPC without `NPCDoNotGainBoost` / `NewGamePlusFakeLevel` gets
  `currentLevel += theGame.params.GetNewGamePlusLevel()` (capped at max player level + 5). The level fact is that number
  (`gameParams.ws:493`: `newGamePlusLevel = FactsQuerySum("FinalNewGamePlusLevel")`, read at game init, i.e. at every load).
* `npc\npc.ws:1260-1262`: bosses additionally get `BOSS_NGP_BONUS` x `NewGamePlusLevelDifference()` (= level fact - 30,
  `gameParams.ws:314-317, 1042-1045`).
* `npc\npc.ws:1522-1523, 1584-1590`: fist fights take the added levels off again.
* `behavior_tree\tasks\combat\btTaskDodge.ws:182-188` and `btTaskPerformParry.ws:158-164`: dodge and parry chance x1.15.
* `engine\sound.ws:486-488`: music threat weight.
* `r4Game.ws:1040-1041`, `gui\hud\modules\hudModuleQuests.ws:526-528`, `gui\menus\journalQuestMenu.ws:578-580`: displayed
  quest / reward levels get the level added.

Player and items:

* `gameplay\leveling\levelManager.ws:234-246`: double XP below level 50 + extra levels.
* `components\inventoryComponent.ws:375-402, 4443-4446, 4503, 4530-4626`: item level and autogen ability counts (`autogen_fixed_*`
  instead of `autogen_*`) change under the fact.
* `components\inventoryComponent.ws:4683-4697, 5036-5081`: every item the player or a merchant receives while the fact is set gets
  `NewGamePlusLevelDifference()` stacks of `autogen_fixed_{steel_dmg,silver_dmg,armor_armor,pants_armor,gloves_armor}`, then the
  modifier `NGPItemAdjusted = 1` (and `DoNotAdjustNGPDLC = 1`). That is the 23 stacks (53 - 30) on `NGP DLC14 Skellige Armor`,
  `Geralt Shirt`, the `Body ...` default items, and 23 + level (70-74) on `Wolf`, `Tlareg`, `Novigraadan sword 1`, `W_Mace01`.
* `container.ws:501-503, 670-790` and `quests\...mq4006_armor.w2phase`: NG+ wolf schematics / armour in chests.

Start of an NG+ (`player\playerWitcher.ws:1095-1260`, `NewGamePlusInitialize`): `SetNewGamePlusLevel(GetLevel())` (writes
`FinalNewGamePlusLevel`, `gameParams.ws:1024-1036`), raises Geralt to level 30 minimum, removes quest items, replaces school / DLC
gear by `NGP ...` variants (`NewGamePlusReplace*`, same name with `NGP ` in front), marks all items `NGPItemAdjusted`.
C++ (not in the scripts) writes the fact `NewGamePlus` and the `NewGamePlus > active` bool in `StartNewGamePlus`.

The game has its own switch: `temp.ws:11466-11473`, console `simngp(flag)`: `FactsAdd("NewGamePlus")` or
`FactsRemove("NewGamePlus")`, then `SetNewGamePlusLevel(0)` (which stores 30 in the level fact).

Quest graphs that mention it (strings in the extracted graphs, wiring not decoded): `quests\part_1\q302_mafia.w2phase` ("If
NewGamePlus", `NewGamePlusFakeLevel`), `q305_replacer.w2phase`, `minor_quests\kaer_morhen\mq4006_armor.w2phase` ("Add NGP armor to
chest"), `dlc\bob\...\sq701_tournament.w2phase` ("NewGamePlus >= 1"), `bugfixing_phase.w2phase` (`W3QuestCond_NewGamePlusEnabled`
-> `theGame.IsNewGamePlusEnabled()`, `quests\bugfixing_phase` / `dlc9_new_game_plus` unlock message), and the prologue (already
played on every template). With the fact gone they take the normal branch (HYPOTHESIS: not decoded node by node).

## 3. Edit plan (implemented behind an opt-in)

1. Remove the facts `NewGamePlus` and `FinalNewGamePlusLevel`. This is what the game's own `simngp(false)` does for the first one;
   the second is only read inside the gates, but a normal save has no such fact and `gameParams.ws:493` then yields 0, so we match
   a normal save. (The game's own `FactsRemove` leaves an entry with zero records; fact removal as a whole was proven in game by
   gate D, `gwent_difficulty`.)
2. Set `NewGamePlus > active` to false. Leave `Enable` alone (false on every NG+ save, which is what a not-yet-finished normal
   game has).
3. Leave every item record as it is (see risks). The report line says how many `NGP ...` records there are.
4. Nothing else changes: `questSystem`, `community`, `universe` (level, items) stay byte-identical.

`w3save::newgameplus::{read, turn_off}`; `start_kit.new_game_plus = false` in a run file; `w3al character-set --no-ngplus`;
`w3al edit --no-ngplus`; `Changes.no_new_game_plus` for the app. The start-kit verifier accepts changes in `facts` and
`NewGamePlus` only when asked and checks that exactly the two facts changed.

Offline results: `turn_off` on all 43 NG+ fixtures: re-reads clean, typed round trip and recomputed table equal, check-save clean,
only the two facts and the one bool differ, every other top-level block byte-identical; the 29 normal saves are left
byte-identical (`crates/w3save/tests/newgameplus_fixtures.rs`, `crates/w3al-core/tests/start_kit_fixtures.rs`). Candidate:
`C:\w3-out\my_skellige\final_nong\ManualSave_8c0ab_7ea51000_50a580a.sav` from `final\ManualSave_8c0ab_7ea51000_4f51969.sav`:
8868 -> 8866 facts, `graph-diff` shows questSystem, community, gameProp, worldState, universe byte-identical.

## 4. Risks and what is not settled

* HYPOTHESIS: the game loads and plays the edited save as a normal game. Not tried in game. Nothing here is unusual for the
  format (a fact removal and a bool flip), but the bool is the less certain part (below).
* HYPOTHESIS: `NewGamePlus > active` is only the engine's own "this run is an NG+" marker. No script reads it by that name
  (`IsNewGamePlusEnabled`, `commonGame.ws:219`, is native; by its name and `EnableNewGamePlus` it is `Enable`). Evidence for the
  meaning is the three save kinds above. If the game re-derived `active` from the fact, clearing it is harmless; if it kept
  `active` and the fact disagreed, the worst case seen is an NG+ label somewhere in a menu.
* Enemy levels already spawned in the save: NPC levels are computed at spawn (`npc.ws:613-623`); we found no `NPCLevelBonus` /
  `NewGamePlusFakeLevel` in the save's blocks, so nothing stale is expected (HYPOTHESIS: native code stores no NPC ability
  lists in `community` / `universe` for transient actors).
* Item records stay NG+-inflated. Weapons carry 70-74 `autogen_fixed_*_dmg` stacks, armour pieces 23 `autogen_fixed_*_armor`
  stacks, plus 8 `NGP ...` DLC pieces. In a normal game they are far stronger than level-appropriate gear and show a level
  above the player's, but nothing reads them as an error. Equipment fixed by the start kit (`gear = "recommended"`) is the
  level-appropriate set. Not done, deliberately: stripping the stacks or renaming `NGP X` to `X` (the base items exist under
  that name in the third-party catalog, lead only: `NGP DLC1 Temerian Armor` level 29 vs `DLC1 Temerian Armor` level 4) needs the
  record's `hash` and ability list rules, which `character-inventory.md` lists as hypotheses. Do it only if the in-game check
  shows the gear is a problem.
* A normal game has `Enable = false` until the story is finished (the `dlc9` graph then sets it). Templates already past the
  ending would be wrong to touch; none is.
* Graph branches that test the fact (section 2: Novigrad `q302`, `q305`, `mq4006`, BoB `sq701`) will take the normal branch for
  every quest that has not run yet on the template. Quests already played on the pack's NG+ keep their NG+ outcome (fake-level
  abilities, NGP armour in the chest); not checked per quest.

## 5. The one in-game check

Load `final_nong\ManualSave_8c0ab_7ea51000_50a580a.sav`. Open the Journal: the quest levels (and enemy skull levels in the first
fight, e.g. a drowner or wolf) must be the ordinary ones, not shifted by +53; the character screen must not offer or mention NG+.
Then save, and `w3al facts <that save> --grep NewGamePlus` must show no `NewGamePlus` / `FinalNewGamePlusLevel` (the game did not
add them back) and `w3al tree --block NewGamePlus` must read `active = false`.
