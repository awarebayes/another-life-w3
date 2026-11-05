# Decision catalogue (v1)

Date 2026-11-05. Offline study, no game. Builds on `decision-chaining.md` (windows, closures, pack numbering) and
`docs/findings/branch-synthesis.md` (rejoining branches). v1 rule: a choice is offered when it changes an ending board, the
Kaer Morhen battle allies, or who lives or dies; romance is added. Hearts of Stone, Blood and Wine and the Witcher 2 import are
out of v1 (`docs/handoff/STATE.md`, `act1-white-orchard.md` section 7).

Labels: **[graph]** = read in the quest graphs (`.w2phase`) or scene files (`.w2scene`) of the user's own 5.01 install with the
scratch CR2W reader (extracts and dumps stay in the session scratchpad, never in the repo); **[save]** = read on real saves with
`w3al facts` / `w3al journal`; **[inferred]** = reasoning from those; **[hypothesis]** = needs the game. Only identifiers are
quoted: quest paths, journal labels, fact ids, block GUID prefixes (8 hex), scene file stems and socket names.
Public quest names are from memory and marked `(?)` where unsure; check them in game before they reach the questbook.

"pack N" = N-th save of the Nexus 2394 pack in `b`,`c` order (`decision-chaining.md`; pack 1 is a finished Blood and Wine save
that sorts first). "Before" = the last pack save where the decision is not made yet (window template candidate; needs a
2-minute 5.01 re-save). "After" = a save that holds the branch.

## Short answer

1. **20 decisions in 9 windows** (23 if the four Kaer Morhen invitations count one by one). 19 of 20 have a pack "before" save,
   and the pack's next save holds one branch of 19 of them (the exception is Emhyr's reward: the pack never visits Emhyr).
   The 5.01 user sessions add the other branches of spirit, Keira and Letho (invite). Status: **done** Keira (KM / Radovid
   synthesized, killed donor) and Letho invite (both donors); **donors exist** spirit (both branches).
2. **Almost every consequence is a fact read at the time it is played** [graph]: the ending boards
   (`quests\videos_scenes\ending_boards_scene.w2scene`), the ending choice (`quests\epilogues\epilogue.w2phase`), Ciri's fate
   (`q502_avallach.w2phase`, a strong/weak count over 5 facts), the battle (`q403_*` phases) and the ally phases of `q402_ciri`.
   The game scripts read only one decision fact (`q110_geralt_refused_pay`, horse appearance in
   `game\vehicles\horse\horseManager.ws` line 75); the ending logic is not in scripts.
3. **8 of 20 decisions rejoin quickly** after instant blocks or are chosen inside one scene with one graph output (fact-only
   difference): D2 (KM / Radovid), D5 (graph only), D7, D8, D9, D15, D19 [graph] and D12 [inferred]. Their second branch can be
   synthesized from one donor like Keira (section 4, column "rejoin").
4. **Cheapest next windows**: W5 Novigrad romance (pack 42: Triss stays, Yennefer; both rejoin, no play), W7 after the battle
   (pack 48: snowball, Ciri to Emhyr, Emhyr's reward; one short play), W9 Skellige act 3 (pack 54: Lara's necklace, Skjall's
   grave). Table in section 6.

## 1. Method (reproducible from the session scratchpad)

* Fact index over all 1,041 quest graphs: every `CQuestFactsDBChangingBlock` (set) and `CQuestFactsDBCondition` (read) with
  its owning block. Scene index over 2,540 base-game quest scenes (`quests\part_*`, `sidequests`, `epilogues`,
  `minor_quests\{novigrad,no_mans_land,skellige,kaer_morhen}`): `AddFact_S` script labels, flow / choice-line conditions,
  named `CStorySceneOutput` sockets. Scene fact ids come from the script block's label (the parameter bytes are not decoded):
  marked "(scene)" below.
* Per decision: the deciding scene block, a forward walk of each output socket through instant blocks (facts, journal,
  reward, phase setter, layers, condition, and/or) to the first waiting block, and whether the sockets meet again
  ("rejoin"); readers of the decision facts.
* Pack mapping: `w3al facts` of all 99 pack saves for 62 decision facts (first save holding each fact) and `w3al journal`
  status of each decision quest per save. Fixtures: `w3al facts` / `journal` on all 68 saves in `C:\w3-fixtures`.

## 2. What reads the decisions

**Ending boards** [graph] (`ending_boards.w2phase` plays `ending_boards_scene`; flow conditions in the scene):

| board group | facts (read in the scene) | branches |
| --- | --- | --- |
| Novigrad, No Man's Land, Emperor | `mq3035_fdb_radovid_dead`, `mq3035_free_temeria` | Radovid alive / Dijkstra (Radovid dead, no `free_temeria`) / Nilfgaard and Temeria (`free_temeria`); Emhyr victorious vs assassinated |
| Skellige | `q206_hjalmar_king`, `q206_svanrige_king` | Hjalmar / Svanrige / else Cerys |
| Ciri | `q502_ciri_dead`, `q110_visited_emhyr`, `mq3035_free_temeria` | dead / empress / witcher |
| Geralt | `q309_triss_lover`, `sq202_yen_girlfriend` | Triss / Yennefer / alone |

**Ending choice** [graph] (`epilogue.w2phase`): `d1977ac3` `q502_ciri_dead` -> `q503`; else `70d57447` `mq3035_free_temeria`
and `7387edbb` `q110_visited_emhyr` -> `q504` (empress); else `q505` (witcher).

**Ciri's fate** [graph] (`q502_avallach.w2phase` #64-#81): `q502_ciri_strong` / `q502_ciri_weak` +1 per factor, survives
(`e8fbfcd6` `q502_ciri_survives`) when strong >= weak, else `47fa5466` `q502_ciri_dead`. Factors: `q110_visited_emhyr` and
then `q110_refused_reward` (strong) or not (weak), nothing if not visited; `q110_calmed_down_ciri` (strong / weak);
`q210_gave_lara_necklace` (weak / strong); `q210_skjall_done` (strong / weak); `q310_ciri_faced_lodge` (strong / weak).
The scene `q502_08a_ciri_into_tower` and the graph `witcher3_structure` read the same facts.

**Kaer Morhen battle** [graph]: `q403_*` phases read `q109_keira_to_km`, `q402_invited_{letho,hjalmar,mousesack,roche,zoltan}_to_km`,
`q202_folan_lives`, `q202_vigi_saved`, `q206_hjalmar_king`. Ally gathering (`q402_ciri.w2phase`, phase `b74543d1`) has one phase
per ally: Mousesack `06e98eb7`, Hjalmar `ab81de6c` (waits on `q202_hjalmar_saved`, `q206_available`, `q208_completed`; branch on
`q206_hjalmar_king` or `q402_cerys_sent_hjalmar`), Roche `173c9b97` (`sq312_ves_died`, `sq312_fdb_ves_end`), Zoltan `01d9fadd`,
Keira `4c3f57db` (`q109_keira_*`, `sq101_date_with_keira_was_failed`), Triss `b7cf0c92` (`q309_completed` / `q309_failed`),
Cerys `1b58e89a` (always refuses: `q402_cerys_refused_km`). An achievement reward needs `q109_keira_to_km` and Roche.

**Velen finale** [graph] (`q107_swamps.w2phase`): reads only the spirit (`q105_will_help_children` at `6ce4fd3b`, `08dab905`;
`q105_evil_spirit_released` at `5976b21e`) and picks `q107_baron_hanged_himself` / `q107_anna_is_monster` vs
`q107_anna_is_crazy` / `q107_burrows_hanged`. No graph or scene in `q107` reads the botchling facts.

**Out of v1, seen in base-game graphs**: `q310_pregeels_new` reads `import_*` (W2) and `q601_geralt_has_demon_mark` (Hearts of
Stone); `bob_persistent_content` (Blood and Wine) reads `mq3035_free_temeria`, `mq3035_fdb_radovid_dead`.

## 3. The pack line [save]

Decision facts first seen (pack pair): botchling buried 6->7, spirit destroyed 8->9 (`q105_will_help_children` only in 8),
Anna crazy + `q107_burrows_hanged` 10->11, Keira date with sex 12->13, Keira to KM 13->14, Letho helped + invited 14->15,
Junior killed 21->22, Lord of Undvik all saved 33->34, `q203_cerys_saved` 34->35, Cerys queen 35->36, Triss kissed 38->39,
Triss lover + Yennefer "just friends" 42->43, four allies invited 43->44, snowball 48->49 (no Emhyr visit), Ciri faced the lodge
51->52, Radovid dead + Roche/Thaler dead 52->53 (Dijkstra boards, Emhyr assassinated), Skjall done (necklace not given) 54->55,
`q502_ciri_survives` 55->56, then `q505` (witcher ending).

Fixtures beyond the pack and the session donors [save]: `C:\w3-fixtures\ManualSave_6b4a7_7e12cc00_4e232cb` (q402 active) and
`ManualSave_8abb3_7e12c000_4000863` (Novigrad) are **another line**: `q309_completed` without `q309_triss_lover` (Triss left),
no Letho; useful only as offline references for synthesized branches. `ManualSave_52586_7ea4c400_1030937` (5.01, `family`
active, q105 not started) is a W1 template candidate earlier than T_spirit. Eight top-level fixtures fail `w3al facts` (bad
UTF-16 in a packed string); their journals show early Velen / Novigrad / Skellige / Blood and Wine saves, not decision pairs.

## 4. Catalogue by window

Columns: **decided at** = graph file and deciding block (scene sockets) and the facts per branch; **read by** = later
consumers (section 2); **rejoin** = yes (instant blocks meet again or one scene output: second branch synthesizable from one
donor) / no (diverging waits: each branch needs a donor); **before / after** = pack saves (and 5.01 fixtures).

### W1 Velen: spirit (template pack 8 = T_spirit; window closes at the Baron finale, pack 10->11)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D1 | `sidequests/q105evilspirit`, The Whispering Hillock | release / destroy the spirit (decides Anna, the Baron, the orphans) | `q105_evil_spirit.w2phase`: scene `de0b3369` (`Output_help_spirit` -> `q105_help_spirit`, `q105_will_help_children` (scene) / `Output_no_pact` / `Output_later`), ritual scene `ba20c454` (`q105_ritual_done_out` -> `d63e2838` `q105_evil_spirit_released` / `q105_betrayed_out` -> heart fight -> `q105_evil_spirit_destroyed`) | q107 (section 2), `character_journal_entries` | no | pack 8 / pack 9 (destroyed); `session_V1` donors `..._900fd9` (released), `..._91a5f1` (destroyed) | donors exist |

### W2 Velen: Keira and Letho (template pack 13 = T_keira; closes at the Kaer Morhen battle)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D2 | `questact2/q109wrapup`, For the Advancement of Learning (?) | Keira to KM / to Radovid / attack | `q109_wrapup.w2phase` scene `d24f649d`: `Keira to KM` -> `b23ada11` `q109_keira_to_km`; `Keira to Radovid`, `Geralt Convinced` -> `005478a5` `q109_keira_to_radovid`; `Attack` -> fight, `q109_keira_defeated` | q402 Keira phase, q403, `q110_kaer_morhen`, `q310_pregeels_new` | KM/Radovid yes (`7159f18a`), attack no | pack 13 / pack 14 (KM); `session_V1` KM `94abd5`, Radovid `97cdc9`, killed `96b66e` | **done** |
| D3 | `sidequests/sq101keira` (?) | go on Keira's date or not (fling) | `sq101_keira.w2phase` scene `24728ecd`: `Output_start_date` (-> `sq101_mount_keira`; later `sq101_keira_had_sex` (scene `sq101_06_sex_keira`)) / `Output_no_date` -> `e08a395d` `sq101_date_with_keira_was_failed` | q402 Keira phase (pause `c38ee051`: date failed -> journal Failure); effect on Keira at KM unconfirmed | no | pack 12 / pack 13 (date) | open; romance only, low priority |
| D4 | `sidequests/sq102letho`, Ghosts of the Past (?) | help Letho or not | `sq102_letho.w2phase` scene `4ad6d8f9`: `NotHelp` -> `sq102_told_letho_not_helping` / `Output`; both -> `d5960a8f`, later phase `2b3921ee` ends in "Lost Letho" (`38334634` / `f19f767a`, `sq102_didnt_help_letho`, Letho despawned) | `sq102_refused_letho_help`; no Letho ally | rejoins at once, diverges later | pack 14 = T_letho / pack 15 (helped); helped donors in `session_V1`/`V2` | not-help missing |
| D5 | same | invite Letho to KM / send elsewhere | scene `ed477b93` (one output; scene facts `q402_invited_letho_to_km` / `q402_told_letho_to_go_elsewhere`, guarded by `sq102_let_headhunters_go`); condition `ff4f19af` -> journal `bff46e8b` | q403, `character_journal_entries` | graph yes, listener no (branch-synthesis.md section 5) | pack 14 / pack 15 (invited); `session_V2` `a4841f` (invited), `session_V1` `9d341b` (elsewhere) | **done** |

### W3 Novigrad: Junior (template pack 21)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D6 | `questact1/q302rescuingdudu`, Get Junior (?) | kill / spare Whoreson Junior | `q302_mafia.w2phase` phase `c0d4ecf5`, scene `df00796c`: `kill` -> journal `deb506c8`, `q302_completed`, scene fact `q302_whoreson_dead` / `let_go` -> teleport, scene facts `q302_whoreson_spared`, `q302_fdb_whoreson_spared` | `q310_pregeels_new`, scenes in q305 / q310 | no | pack 21 / pack 22 (killed) | open |

### W4 Skellige: Undvik and the throne (template pack 33; D11 bridged through pack 35)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D10 | `questact1/giant`, The Lord of Undvik | Folan lives / dies; Vigi saved / dead; Hjalmar saved | `q202_giant.w2phase`: scene `98ac08c2` (`attack_trolls` / `talked_with_trolls_folan_lives` / `..._folan_dies` -> `q202_folan_boiled`); `q202_folan_lives` (scene `q202_05c_3_trolls_cooking_folan`, also `a2aeb601`); cage scenes `e485db7a` (-> `aaab9c90` `q202_vigi_dead`) / `4cdfd32c` (`out_no_hjalmar` -> `a815a714` `q202_vigi_saved`), which one plays depends on the earlier party (`q202_bring_hjalmar`, `q202_folan_party`) [inferred]; `a29709f5` `q202_hjalmar_saved` | q403 (Folan, Vigi), q402 Hjalmar phase | no | pack 33 / pack 34 (all saved) | open |
| D11 | `questact2/q206berserkers`, King's Gambit | help Cerys / help Hjalmar / refuse (and skipping the quest) | `q206_berserkers_attack.w2phase` scene `31f4501a`: `Cerys` -> `q206_helping_cerys` / `Hjalmar` -> `q206_helping_hjalmar` / `NoThanks` -> `7315dda2` `q206_svanrige_king`; trials `d2fd8c1e` -> `120d0846` `q206_cerys_queen`, `a333f087` -> `7468f59e` `q206_hjalmar_king`; not done by act 3 -> `q210_precanaris` `a1a63215` sets `q206_svanrige_king` | ending boards, q402 Hjalmar, q403, q208, q210 scenes | no (`NoThanks` ends at once) | pack 35 / pack 36 (Cerys) | open |

`q203_cerys_saved` (Possession, pack 34->35) is a completion fact, not a branch: it gates the Cerys ally talk and lines in
q210 / q501; no v1 choice.

### W5 Novigrad: romance (template pack 42)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D7 | `sidequests/sq301triss`, A Matter of Life and Death | kiss Triss or not | `sq301_dawn_estate.w2phase` scenes `6397221b` / `fd10b44f` (one output; scene fact `sq301_kissed_triss`) | only scene conditions in `q309_07_dijkstra_triss_left` and `sq301_10_apology` | yes (fact only) | pack 38 / pack 39 (kissed) | fold into D8 (kiss matters only through q309) |
| D8 | `questact2/q309mages`, Now or Never | ask Triss to stay / let her go | `q309_casablanca.w2phase` phase `3d585dd4`: scene `fd4f5c1e` (one output; scene facts `q309_asked_triss_to_stay`, `q309_told_triss_you_love_her`) -> scene `d6a0ac74` (conditions on those and `sq301_kissed_triss`): `triss returned` -> journal `26851344`, `b50464e9` `q309_triss_lover`, reward, `1c8fefda` `q309_triss_stayed` / `geralt alone` -> journal `58a34027`; both -> `a004eadf` | ending boards, q310, scenes of q403 / q502 / q504 / q505 | **yes** | pack 42 / pack 43 (lover); "alone" on another line: `6b4a7_..._4e232cb` | open, synthesizable |
| D9 | `sidequests/sq202yen`, The Last Wish | Yennefer: love / just friends | `sq202_yen.w2phase` scene `b0225ae2` (one output; scene facts `sq202_yen_girlfriend` / `sq202_yen_just_friends`) | ending boards, `q401_konsylium`, q310, q504 | **yes** (fact only; character-journal listeners to check) | pack 42 / pack 43 (just friends) | open, synthesizable |

### W6 Brothers in Arms (template pack 43)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D12 | `questact2/q402{skellige,novigrad,velen}allies` | invite Ermion (Mousesack), Hjalmar, Roche, Zoltan (4 choices) | `q402_ciri.w2phase` ally phases (section 2): context talks `4075a745` -> `e7faac69`, Hjalmar -> `2b758a00`, `4a92523c` -> `8b4e40e6`, `22f5025a` -> `2fa97832` (`q402_invited_*_to_km`); not inviting = the phase keeps waiting | q403 (all), q208 | each ally is its own phase: "not invited" = the closure without that phase's ops [inferred] | pack 43 / pack 44 (all four) | **done 2026-11-05**: template synthesized from pack 43 by `ugly_baby.toml`'s `done_effects`, four synth effects in `brothers_in_arms.toml` (Hjalmar through Cerys `0cd4179d` + `c9e4e916`), exports VERIFIED for all four / none / two subsets; `docs/research.md` |

Keira, Letho and Triss come from D2, D5, D8; Hjalmar also needs D10 and D11.

### W7 After the battle (template pack 48)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D13 | `questact3/q110postbattle`, Blood on the Battlefield | snowball fight / talk | `q110_kaer_morhen.w2phase` scene `4002a0f6`: `Snowball` -> phase `b7a640fb`, `63fff3f2` `q110_calmed_down_ciri` / `Talk` | Ciri's fate | no | pack 48 / pack 49 (snowball) | open |
| D14 | same | go with Ciri to Emhyr or not | scene `a9e9be4a` / `8e9024bd`: `Emhyr` / `Imlerith` / `Wait`; top phase `685fc645` exits `Vizima` -> phase `938bd2d4` (`3a8fbae0` `q110_visited_emhyr`) or `NML` | Ciri's fate, ending choice, ending boards | no | pack 48 / pack 49 (not with her) | open |
| D15 | same | accept / refuse Emhyr's pay (only after D14 = Emhyr) | `q110_vizima.w2phase` scene `aea4283d` (one output; scene facts `q110_accepted_reward` / `q110_refused_reward`, `q110_geralt_refused_pay`); fact listeners `1af99c14` / `26b11e5d` -> journal | Ciri's fate; horse appearance script | **yes** | pack 48 / none | open, needs one Emhyr donor |

### W8 Novigrad act 3 (template pack 51; D17-D18 bridged through pack 52)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D16 | `questact3/q310pregeels`, Final Preparations (?) | let Ciri face the lodge alone / help | `q310_pregeels_new.w2phase` phase `54b09c2d`, scene `d445d2b7`: `out no help` -> `3e6f7b3b` `q310_ciri_faced_lodge`, scene `cb2fe27e` / `out help` -> scene `2dc2f9bb` | Ciri's fate | no (short) | pack 51 / pack 52 (faced) | open |
| D17 | `minorquests/mq3035_emhyr`, Reason of State | join the plot against Radovid or not | `mq3035_meet&bait.w2phase` scene `51cf714f`: `mq3035_agrees_to_plan` / `mq3035_disagrees_to_plan` | ending boards, ending choice | no | pack 52 / pack 53 (agreed) | open |
| D18 | same | side with Dijkstra / with Roche | `mq3035_wrap_up.w2phase` scene `de41f700` (`Fight_output` / `Leave_output`); `65fb9638` `mq3035_fdb_dijkstra_dead` + `c297b437` `mq3035_free_temeria` / `0026e3bd` `mq3035_fdb_roche_talar_dead`; `mq3035_novigrad_roche.w2phase` `85114cb9` `mq3035_fdb_radovid_dead`, `a35e36a4` `mq3035_free_temeria` (via `cad5d728` `mq3035_fdb_talar_path`). Socket -> fact mapping not traced past the first waits **[unconfirmed]** | ending boards, ending choice, `cg300_novigrad`, traders | no | pack 52 / pack 53 (Radovid dead, Roche/Thaler dead) | open |

### W9 Skellige act 3 (template pack 54)

| # | quest | choice | decided at | read by | rejoin | before / after | status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| D19 | `questact3/q210avallachlab`, Child of the Elder Blood (?) | Lara's necklace / Ciri wrecks the lab | `q210_precanaris.w2phase` phase `8b69cc69`, scene `c6890bb5` (one output; scene facts `q210_gave_lara_necklace` / `q210_cs_ciri_destroy_lab`); condition `1efc2270` -> layers `3940d2b4` -> rejoin `02a48b7e` | Ciri's fate | **yes** | pack 54 / pack 55 (no necklace) | open, synthesizable |
| D20 | `questact3/q210skjallgrave` | visit Skjall's grave or not | phase `36d46dd1`, scene `b553b871` (scene fact `q210_skjall_done`) | Ciri's fate | n/a (optional visit; "not" = phase never played) | pack 54 / pack 55 (visited) | open |

## 5. Traced but not offered in v1

* Botchling (`questact1/family`: `q103_buried_botch` / `q103_killed_botch`): read only in `q103_daughter` and the `sq108` despawn
  phase [graph]; not by q107.
* Anna's doll in q107 (`b619d105`: `Output_cut_sigil` / `Output_failed` -> `q107_anna_disenchanted` / `q107_anna_burnt`):
  spirit-released line only, Anna dies either way.
* Outcomes, not dialogue choices: Lambert (`q403_first_square` `c4281b08`: not `q403_geralt_saved_lambert` -> `q403_lambert_dead`,
  read by q403 / q110), Ves (`sq312_kill`: `sq312_ves_died` when the player is not near in time; read by q402 Roche and mq3035).
  The pack has neither death.
* `q301_godling_sara_peacefully` (read by q111 / q311 only), Triss failing the mages (`q309_failed` -> Triss phase Failure; no
  pack or fixture save has it).

## 6. Priority and count

The template of a later window encodes the pack line's answers for earlier windows. Every far consumer in section 2 reads facts
when it runs, so an earlier answer whose consumers all lie ahead might be applied to a later template as facts plus character
journal entries [hypothesis; the journal is append-only, `decision-chaining.md` point 1]. That is the open design question
before W6-W9 can serve more than the pack line's earlier answers.

| rank | window | decisions | branches in hand | to capture | user play [estimate] | why |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | W5 Novigrad romance (pack 42) | D8, D9 (D7 folded) | Triss lover, Yen friends (pack 43) | re-save packs 42 and 43; synthesize Triss "alone" and Yen "love" | 0 (2 re-saves) | two ending boards; both rejoin |
| 2 | W7 after the battle (pack 48) | D13, D14, D15 | snowball, not with Ciri (pack 49) | play `Talk` (~5 min) and `Emhyr` (~15 min; gives one reward branch, synthesize the other) | ~20 min | three Ciri-fate inputs, one ending choice, scopes end in q110 |
| 3 | W9 Skellige act 3 (pack 54) | D19, D20 | no necklace, Skjall visited (pack 55) | synthesize the necklace; "no grave visit" = closure without phase `36d46dd1` [inferred] | 0-10 min | two Ciri-fate inputs |
| 4 | W8 Novigrad act 3 (pack 51) | D16, D17, D18 | faced lodge, plot joined, Dijkstra side | play `out help` (~10 min), refuse the plot, Roche side (~30-45 min each) | ~1.5 h | all war boards and the empress ending |
| 5 | W4 Skellige (pack 33) | D10, D11 | all saved, Cerys | `NoThanks` (~5 min), Hjalmar path (~1 h), Undvik deaths (~30 min) | ~1.5 h | Skellige board, two allies |
| 6 | W6 Brothers in Arms (pack 43) | D12 x4 | all invited | synthesize "not invited" per ally | 0 | battle allies; needs W2/W4/W5 lines first |
| 7 | W3 Junior (pack 21) | D6 | killed | play `let_go` (~10 min) | ~10 min | one life, minor consumers |
| 8 | W2 rest (pack 12-14) | D3, D4 | date, helped | play no date (~5 min), Letho not helped (~20 min) | ~25 min | D2, D5 done |
| 9 | W1 spirit (pack 8) | D1 | both donors | in-game gate only | 0 | done offline |

**Total: 20 decisions (23 player choices with the four invitations), 9 windows.** Before-saves: 19 of 20 in the pack (D15
depends on D14). After-saves: pack branch for 19; second branch in hand for D1, D2, D5; 8 decisions rejoin or differ by scene
facts only (short answer point 3). Done: 2 (D2, D5). Donors exist: D1. All pack saves are old-gen: each template and pack
"after" save needs a 5.01 re-save before capture.

## 7. Base-game decisions added 2026-11-05 (content C; facts traced in the graphs, walked with `w3al synth-walk`)

| decision | quest file | template | branches in the questbook | status |
| --- | --- | --- | --- | --- |
| Payback (Dijkstra's other leg) | `06_act3/final_preparations.toml` choice `dijkstra`: scene `fc64e6ac` `out convinced` (`q310_dijkstra_spared`) / `out broken leg` (`q310_dijkstras_other_leg`) | pack 51 (5.01) | spared, leg; the lodge choice now starts at the parked scene `d445d2b7` | exports VERIFIED; spared donor-checked vs pack 52 |
| It Takes Three to Tango | same file, derived: `Girl Talk` `8c5a1fa5` reads `q309_triss_stayed` and `sq202_yen_girlfriend`; trap scene `e056f87b` -> `q310_triss_yennefer_payback` | pack 51 | no own choice; both romances in the run take the trap path (stops `4dd402ab`, `8fc17d40` keep `q310_has_wine`) | exports VERIFIED on a scratch template with `sq202_yen_girlfriend` (graph-only) |
| Ruler of Skellige | `04_skellige/kings_gambit.toml` | pack 35 (5.01) | Hjalmar (new walk, coronation parked), Svanrige (`NoThanks`, exports now), Cerys = play | Cerys blocked by walker (see research) |
| Vigi | `04_skellige/lord_of_undvik.toml` choice `vigi` | pack 33 (5.01) | dead (`q202_vigi_dead`, walk from `054edfa3`), play | saved not expressible (race between waits) |
| Carnal Sins | `03_novigrad/carnal_sins.toml` scene `39a73bb3` | pack 29 old-gen (re-save needed) | not guilty, kills anyway (donor pack 30: 51/0/0), guilty | exports VERIFIED on the old-gen stand-in |
| A Towerful of Mice | `02_velen/a_towerful_of_mice.toml` scene `56a4375a` | pack 11 old-gen (re-save needed) | help (`q106_morowa_released`), graham (kiss, donor pack 12 hits), later | exports VERIFIED on the old-gen stand-in |
| Lambert, Ves, botchling, Sara, A Greedy God and the rest | not written | see `docs/research.md` 2026-11-05 "content C" | | open |
