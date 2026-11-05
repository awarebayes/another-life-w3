# Applying several act-2 decisions to one save (decision chaining)

Date 2026-11-05. Offline study, no game. Labels: **[proven]** = shown on real saves by a test or a command below,
**[inferred]** = reasoning from proven data, **[hypothesis]** = needs the game.

Saves: the Nexus 2394 pack (one old-gen playthrough, 99 readable saves, ordered by the `b`,`c` parts of the file name;
"pack N" = N-th in that order, inventory `scratchpad\nx\inventory.txt`), the user's 5.01 re-saves T_spirit (pack 8),
T_keira (pack 13), T_letho (pack 14) and donors played from them (`C:\w3-fixtures\session_V1`, `session_V2`,
`BRANCHES.txt`). Two "null" saves (made from a template with no quest change: `session_V2\ManualSave_6b9be_..._a52090`,
`session_V1\ManualSave_568ef_..._94086d`) measure noise. Pack line: spirit destroyed (pack 8 -> 9), Baron hanged / Anna
crazy (10 -> 11), Keira to Kaer Morhen (13 -> 14), Letho invited to Kaer Morhen (14 -> 15) (facts read on the pack saves).

New tools (this study): `w3al stubs SAV`, `w3al graph-diff --stub-id`, `w3al graph-check --toml D [--base T] SAVES..`
(per op: B applies / A already done / - absent / x different; with `--base`: quest nodes a save has in the delta's scopes
beyond the template's, and stubs present under another idTag). Tests: `crates/w3save/tests/stub_identity_fixtures.rs`.

## Short answer

1. A captured decision applies only to a template where its quest is **before** the decision. Sibling branches always
   differ by journal entries the other branch lacks (1-2 removed per side in all three decisions), and the journal is
   append-only, so a decision cannot be switched after the fact [proven offline; removal never tried in game].
2. A decision delta captured on template T can be moved to an **earlier** template only as a **closure**: the delta from
   that earlier template to the donor, which also removes the quest's start triggers that still wait there [proven offline
   for Letho: T_keira -> Letho donor composes with the Keira decision on T_keira].
3. A decision cannot be moved to a **later** template once later main-quest play consumed its quest's scopes (the spirit's
   phase scope is gone after the Baron finale, pack 10 -> 11) [proven offline].
4. Stubs are addressed by `idTag` (`community/stubs/a{idTag=..}`): unique per save and never reused for another spawn-set
   entry in 265 saves [proven]. The "churn in hundreds of stub slots" was mostly `processingTimer` (every stub's clock) [proven].
5. Recommended model: **decision windows**. The act is cut where a decision's consequences end; one template per branch of
   earlier windows, parked before every decision of its window; the window's decisions are closures from that template
   (section D). The next in-game gate: T_keira + Keira (to Kaer Morhen) + the Letho closure (invited).

## A. Attribution [proven unless marked]

Method: sibling donors (two branches from one template share everything except the decision), the template, the null
saves (noise with no quest change), and lifetimes of each node across the 98 consecutive pack pairs (`w3al tree --grep GUID`
per save). "Scope" = the innermost `questThread[GUID]` above a change. `Q` = `quest[witcher3_quest.w2quest]/questThread/
questThread[7b125918]/questThread`.

**Noise** (changes in the null saves and in many unrelated pack pairs): timers `5e9d5fdf` (61 of 98 pack pairs), `b4188b3b`
(44), `3eed44ca`; toggling scopes `09366804` (33; also in the T_letho null save), `4cd34248` (24, `inputName` flips),
`4056cc99`, `cd60e348`, `7f4ccee7` (9, alternates `1fad9a65`/`fecafed0`), `bob_quest > abead12a` (73) and `276c5a3b`; the
scene `sq104_villager_1`; reorders of `b531a276` and of the scene list. Facts: 23 load-stamped ones (`*_noticeboard_empty`,
`InteractiveEntity_empty`, `game_is_loaded`, ...), dialogue ids `N_N_N`, `*_weapon_hit*`, `actor_*_was_killed*`, statistics.
Community: `processingTimer` of every stub (433-438 per pair), routine moves (position / action point) of unrelated NPCs,
crowd respawns (`mq1007_drunk_men`: stub removed + new stub with a new idTag), `mh305_merchant` set, blacksmith schedule.
Hub scopes hold many quests' children and change often (`117e4006` 35 pairs, `279a1a03` 23, `b531a276` 14): only point
ops on the quest's own children may touch them.

| | Spirit (T_spirit -> released / destroyed) | Keira (T_keira -> KM / Radovid / killed) | Letho (T_letho -> invited / elsewhere) |
| --- | --- | --- | --- |
| own scope | `Q/questThread[a0881c9f]/questThread[0edad8e4]` (q105; pack lifetime 5-10). Released: `723f020d` -> `470d5d3b`, `623dadd5 > 53a6d16e` leaves. Destroyed: `11b7687f`: `24858186` -> `1f5f6045` | `a536c75d` (under `37028f15/143c1d62`): `b3a72321` -> KM `f5290e77` (pack 14-47) / Radovid `c133524a`; killed: the whole thread `a536c75d` leaves `143c1d62` | `b8f04d50` (under `117e4006`): `b64ee02e` leaves; `974e8877`, `d8b0024d` (+ `4c121657` invited only) |
| decision's long wait | released: `279a1a03 > 059fd2dc` (alive pack 2-99 on the destroyed line) is removed | KM / Radovid wait above lives to the Kaer Morhen battle (pack 47) | invited: `279a1a03 > 6850c728` keeps a new wait `7573f7fb` (pack 15-48); elsewhere: thread `6850c728` removed |
| listeners | `057e0897` (`b9ea9a8e`,`c14d7770` -> `1171cb29`,`d9d4fa45`), `f89fdbce`; destroyed also `suspendedScopesData` of `61545c9b`, `f0b52e98` | `117e4006 > 7ffa6436` leaves; `20babdd8 > 973a4357` (`inputName` Restart, 2 blocks out, `ad92bd6b` in) | `c6d000ad` (3 blocks -> `38205cbc`), `26dde217` (`d9c54a12` -> `ecc26888`, invited only; elsewhere still waiting [inferred: saved earlier]), `b531a276`: `b48b318d` -> `980d81e0` |
| scene list | `burrows_soltis` guid replaced | `keira` removed | `letho` added |
| journal | 8 flips + 2-3 appends; branches differ: released `q105evilspirit/41f4aff8/7a4064dc` + `duchdrzewa/a68f61a9`, destroyed `.../c018bbf0` | KM / Radovid: 1 flip + 4 appends, differing in 2 entries each side; killed: 5 flips + 5 appends | 7 flips + 5 appends; branches differ in 4 entries each side |
| facts | `q105_evil_spirit_released` / `q105_betray_spirit` + `q105_evil_spirit_destroyed`; `q105_evil_spirit_done`, `q105_ritual_done`, `q105_deactivate_soltis_dd` | `q109_keira_to_km` / `_to_radovid` / `q109_keira_defeated`; `q109_confronted_keira`, `keira_part1_dd_stop` (-`_start`) | `q402_invited_letho_to_km` / `q402_told_letho_to_go_elsewhere`; `sq102_helped_letho`, `sq102_letho_completed`, `sq102_hide_community`, `sq102_explode_building`; invited also `sq102_let_headhunters_go`, `sq102_letho_lost_medallion` |
| spawn sets | +`e3_burrows_npcs`; -`q105_evil_heart`; released also +6 `q105_*` children/ghost sets; `nml_burrows_soltis` phase | `keira_metz` phase `q109_island_attacking` (killed: set removed) | -`letho`, -`sq102_letho_horse`, -`sq102_midcots_boy`; elsewhere +`sq102_midcots_headhunters` |
| stubs (by idTag) | removed: the soltis and evil-heart stubs; added: released 10, destroyed 4 (new idTags per branch) | Keira's stub: phase entry, appearance, state | 3 removed (Letho, horse, boy) + 2 changed (Dolores, evictor) |

The game also drops the `universe > world[..] > layerStorage > entityData` entry of a removed stub: T_letho -> invited
removes exactly the 3 entries whose idTag is the 3 removed stubs'. Gates G/P/F/R/M left these entries in place without
visible harm.

## B. Stub identity [proven offline]

`community > stubs > a` holds `storyPhaseLocation {community (spawn set file), entry, storyEntry, entityEntry}`, `idTag`,
`position`, `state`, `appearance`, `processingTimer`, ... Evidence (`stub_identity_fixtures.rs`, `W3AL_FIXTURES` =
`C:\w3-extract` and `C:\w3-fixtures`):
* idTag unique in every save: 201 + 64 saves, 117,940 + 20,019 stubs; entityData idTags unique too.
* An idTag never names two spawn-set entries (file + `entityEntry`) across all 201 saves (10,215 idTags). Appearance is
  mutable (5,575 changes), so it is not part of the identity.
* A respawned NPC gets a new idTag: Letho is `0128b992..` on T_spirit/T_keira and `01a2e954..` on T_letho (respawned at
  quest start). (spawn set, `entityEntry`) is unique for named NPCs (4 duplicate groups per Velen save: crowds such as
  wolves) and finds the respawned stub; `graph-check --base` reports it.
* Positions are stable within a line except for removed slots and the list end: 442 of 450 common stubs keep their index from
  T_spirit to T_letho, but the decision NPCs are exactly the ones near the end (Dolores is `a#473`).
* Replaying the stub-identity diff of `questSystem` + `community` of each consecutive lukwraith pair on A: 86 pairs, 74
  byte-exact, 12 equal up to stub order; 2,242 stub ops by idTag vs 3,508 by position.

Implemented (`w3save::graph`): `SELECTORS` (`stubs > a > idTag`, `layerStorage > entityData > idTag`), selector path steps
`a{idTag=<hex>}` resolved by `locate` (so every op and the TOML guards accept them), `DiffOptions { stub_identity }` /
`diff_with` / `diff_top_with` (stub-list order not reported: **[hypothesis]** stub order carries no meaning),
`ops_from_diff` turns a removed stub selector into `swap-remove` (what the game does), `stubs()` (`StubInfo`),
`child_steps()`. CLI: `w3al stubs`, `graph-diff --stub-id`, `graph-check`. `graph_effect::step_name` accepts `{`.

## C. Portability [proven offline unless marked]

`graph-check` of the deltas (`--stub-id` captures, point ops on hubs) on other saves; numbers are B/A/-/x of the ops.

1. **Onto earlier templates.** Letho invited (39 ops, captured on T_letho) on T_keira: 15/14/9/1. All inserts apply, the
   listener scopes `c6d000ad` and `26dde217` are already in their post-quest state (A), the Dolores stubs do not exist yet
   (-), but the scope drift shows the quest's **start triggers still waiting** on T_keira and T_spirit: `6850c728 >
   72a761d5`, `b8f04d50 > 15601b24`, `8fdb44ab` (pack lifetime 5-13, gone when the quest starts) and `2aebb444` missing.
   Applied as is, the quest would start again (the gate J failure) [inferred]. The **closure** captured T_keira -> Letho
   donor (34 ops: the same scopes plus the triggers, 2 scene entries, 6 spawn sets, Letho's T_keira stub, 2 stubs; donor
   names are not a prefix: remap) applies 34/0/0/0 on T_keira and 33/0/0/1 on the Keira donors and T_spirit.
   Keira (19 ops, T_keira) on T_spirit / pack 11 / pack 12: the Keira chain is not there yet (`a536c75d` waits at other
   blocks: `1e5b960d`, `37497243`; `117e4006` has 4-5 other q106 threads): a Keira closure from those saves = the Keira
   chain q106findkeira/q106tower/sq101/q109.
2. **Onto later pack saves where the decision is not made:** none exist; each decision is made in the pack pair right
   after its "before" save. Later saves hold the opposite choice for the user's other branches, i.e. switching (point 1 of
   the short answer).
3. **Combination.** On T_keira: Keira to KM (prefix) + Letho closure (remap) compose: all copied nodes equal their donors
   (resolved fingerprints), the triggers are gone, nothing outside the two deltas changed
   (`keira_decision_and_letho_closure_compose_on_t_keira`). Adding the spirit is not possible on T_keira or T_letho: its
   phase scope `0edad8e4` is absent there (consumed by the Baron finale, pack 10 -> 11, where `q107_burrows_hanged` /
   `q107_anna_is_crazy` appear), and "released" would have to remove destroyed journal entries. On T_spirit, Keira and Letho
   would need the whole Velen main path as closures: T_spirit -> T_keira = 179 graph changes in 36 non-noise scopes and
   161 journal appends (q106tower, q108eve, family, sq101keira, q107swamps, q105witches, ...).

Main-quest prerequisites [hypothesis]: the Letho closure marks sq102 done while q109 is still active (in the pack it started
after the Keira decision); nothing in the graph refers to it once its triggers are removed, but only the game can show that
q109 and the Kaer Morhen waits (`f5290e77`, `7573f7fb`) behave.

## D. Recommendation

**Content model: decision windows on one spine** [inferred from A-C].
* Cut the act where a decision's consequence closure ends. Act 2 Velen: W1 = spirit, closed by the Baron finale (pack 8 ->
  11); W2 = Keira + Letho (independent scopes; both end in waits for the Kaer Morhen battle, pack 47).
* Per window one template per branch of the earlier windows, parked before every decision of its window: W2 needs
  T_w2[destroyed] (pack 11-13 re-saved: T_keira qualifies) and T_w2[released] (to be played).
* Each decision delta of a window is a closure **from the window template** to the branch donor (bridged through the pack
  line when the donor was played from a later template, as Letho). Capture once on the spine; port to the other templates
  of the window by `graph-check --base` (all B, no drift) [hypothesis for T_w2[released]: the scopes are disjoint from the
  spirit's, so it should hold].
* The export stops before any closure that merges decisions (the Kaer Morhen battle consumes Keira's and Letho's waits);
  the game plays those consequences itself from the inserted waits [hypothesis].

**Capture work for the user:** re-save pack 11 in 5.01 if a window template earlier than T_keira is wanted (2 min); play
"spirit released" from the existing donor through the Baron finale and park at the T_keira point (q109 decision) to make
T_w2[released]: about 2-3 h (pack 9 -> 13: family, Ladies of the Wood, Return to Crookback Bog, q106 and sq101; 1-1.5 h if
the window template is parked right after the Baron finale and the Keira chain becomes part of Keira's closure). Optional:
finish q109 after the KM / Radovid decisions (5 min each; the current donors stop with q109 Active). Keira and Letho need
no new play.

**Exporter changes:** (1) templates keyed by earlier-window answers (questbook: per act a list of templates with the answers
they encode); (2) graph effects with selector paths (`a{idTag=..}`; works today through `locate`) and point ops on hubs;
(3) remap on by default for act 2 (closures copy `Vector`, `SActionPointId`, `IdTag`, `ActivatedInputs`: untested in game);
(4) refuse an effect when `--base` drift shows quest nodes in its scopes that it does not cover, and treat a remove of an
absent node as done only when the node is not elsewhere under another idTag; (5) optionally drop the universe `entityData`
of removed stubs (needs `universe` in `GRAPH_BLOCKS`; not needed so far).

**Smallest in-game gate (X)** (describe only; not built): X1 = T_keira + Keira to Kaer Morhen (journal 4 appends + 1 flip,
facts `q109_keira_to_km`, `q109_confronted_keira`, `keira_part1_dd_*`, graph/community 19 ops from donor 94abd5) + Letho
invited closure (34 ops from donor a4841f with remap, 28 `sq102*` journal appends + the Letho character entries, the
`sq102_*` / `q402_invited_letho_to_km` facts). PASS: loads; q109 continues to the KM outcome; sq102 and sq102dolores show
completed; Letho is not at Dolores' farm and the quest does not start when Geralt goes there or to the notice boards;
re-save keeps `7573f7fb`, `f5290e77` and has no `72a761d5` / `15601b24` / `8fdb44ab`. Control X0 = donor KM unchanged.
Optional negative control X2 = X1 with the delta captured on T_letho (triggers left): expected to restart sq102.
Offline the graph part of X1 was built and verified in the scratchpad (`chain\out\c2_TK_keira_letho.sav`, `check-save` clean).
