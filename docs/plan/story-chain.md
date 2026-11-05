# Story chain: one save for the whole story, with earlier decisions rewritten on a later template

Date 2026-11-05 (night). Labels: **[proven]** = shown on real saves by a test or command named here (offline unless
"in game"), **[likely]** = strong evidence, not shown in game, **[hypothesis]** = needs the game. Builds on
`decision-chaining.md` (windows), `decision-catalogue.md` (who reads each decision, section 2), `no-human-pipeline.md`
(synthesis). Code: `crates/w3al-core/src/templates.rs` (registry, choice), `crates/w3al-core/src/chain.rs` (references,
delta, rewrite, story export, scan), `w3save::journal::remove`; CLI `w3al chain-plan`, `w3al template-scan`, `w3al export`
without `--template`; app command `template_plan`, Welcome / Export screens. Registry: `questbook/templates.toml`
(schema in `questbook/SCHEMA.md`). Tests: `crates/w3al-core/tests/story_chain_fixtures.rs`, unit tests in
`templates.rs`, `chain.rs`, `journal.rs`. Cross-checked against a third-party editor's data (not copied).

## Short answer

1. **Template choice is automatic.** The registry lists ten window templates on the pack line (pack 8 ... 54) with the
   earlier answers each one holds (`encodes`, derived by `w3al template-scan`) and the earlier answers it cannot change
   (`fixed`, with the reason). A run gets the latest template at or before its stop quest whose earlier answers are the
   player's or rewritable, and whose file is found in the template folders (run `template_dirs`, the app's Templates
   folder, default `templates` next to the questbook). A picked save stays a manual override.
2. **Earlier decisions are rewritten after the fact** by carrying the difference between the two answers, measured on
   the decision's own window template (both answers exported there by the normal pipeline), onto the later template:
   decision facts, quest-log entries (removed / appended / status), objective counters, and the decision's quest-graph and
   spawn-set state where it is still live. **Validated on every pair of played branches we have** (spirit released /
   destroyed, Keira Kaer Morhen / Radovid, Letho invited / elsewhere; both directions): **0 decision-fact differences,
   0 journal-entry differences, 0 wait differences** against the played save of the other branch [proven offline].
3. **Full story on pack 54** (stop at Child of the Elder Blood; Triss let go, Yennefer love, talk instead of snowball,
   ride to Emhyr and take the pay, help Ciri with the lodge, all differing from the pack line): exported **VERIFIED**
   (`check-save` clean, round trip PASS) by `w3al export` and the fixture test [proven offline; not loaded in game].
4. **What cannot be rewritten** is a decision whose consequence a later quest already played in the template: the spirit
   after the Baron finale, Keira / Letho / Folan / the allies after the battle of Kaer Morhen, Junior after Final
   Preparations, Reason of State after it happened, Dijkstra's leg after Reason of State, the King's Gambit side after the
   coronation, Keira's date (no 5.01 pack 12). For those the choice falls back to an earlier template and the app lists the
   quests that are then not in the save (section 3).

## 1. Registry and choice

`questbook/templates.toml`: `[[template]] id, stem, line, quests, source, [template.encodes], [template.fixed]` and
`[[line]] quest, line, note` for quests without a usable window template. `line` = the pack save number (the line is
the pack playthrough; `PACKS.txt`, `BRANCHES.txt`). Questbook order and line order may disagree (Skellige act 2 is
pack 33-35, the Novigrad romance pack 42, while the questbook orders Novigrad first): the choice uses the line.

`templates::choose` (unit-tested; `shipped_registry_choices` on the real registry):
* stop quest = `stop_after`, else the last quest the run completes or starts; stop line = its line;
* candidates = templates with `line <= stop line`, latest first; for each, every quest on an earlier line is *baked*:
  `Same` (the player's answers equal `encodes`), `Rewrite`, `Fixed` (a `fixed` key `quest`, `quest.choice` or
  `quest.choice=option` matches a differing choice, or the run does not complete a quest the template has done) or
  `Unchecked` (not recorded: placeholder act-1 quests, quests without a 5.01 template);
* the first candidate with no `Fixed` and a found file wins; later candidates are reported as *passed over* with the
  reason; quests of the run up to the stop that lie after the chosen template are reported as *beyond* (not in the save).

Examples on the shipped registry: the full story above -> pack 54 with 4 rewrites; the same with Junior spared -> pack 51,
Child of the Elder Blood beyond; the spirit released -> T_spirit (pack 8), everything later beyond; stop at Brothers in
Arms with Keira to Radovid -> pack 43, Keira + romance rewritten.

## 2. The rewrite

For each earlier window with a `Rewrite` (`chain::rewrite_all`, line order):
1. **References** (`chain::references`): the window's quests exported on the window's own template twice, A = the
   answers the chosen template holds, B = the player's (`resolve_only` restricted to the window, the normal
   `export::build_with_sources`: synth walks, donor graph effects, journal tables, facts).
2. **Delta** (`chain::window_delta`): facts whose value differs (noise facts excluded), journal entries only in A / only
   in B / with another status, counters, waits and scopes only in A / only in B, waits both created (the common
   aftermath), spawn-set phases.
3. **Apply** (`chain::apply_delta`) on the later save, then verify (only the planned facts and entries changed,
   untouched top-level blocks byte-identical, table / post-order):
   * facts: A-only removed; B's set (a later change of the same fact in the template is kept as an offset); a fact
     later play removed, or set without A, is left and reported;
   * journal: A-only entries removed with `w3save::journal::remove` (node out, `Size` down, order kept: what the game did
     in gate Y2, docs/research.md); B-only entries appended (Active ones under a quest root the template has finished are
     written as Success, the synthesis rule); changed entries set when the template still holds A's status; counters
     likewise;
   * quest graph and spawn sets: waits are sorted by file into *own* (the decision's quest files), *listeners*
     (`character_journal_entries`: one per decision fact) and *world hubs* (persistent content, default dialogues, bug-fix
     phases, merchants, `witcher3_structure`). The window is **live** when one of A's own waits or a common-aftermath wait
     still waits in the template. Listeners of A still waiting are always swapped for B's (Keira's Kaer Morhen listener
     `f5290e77` lives until the battle: on pack 43 it becomes the Radovid listener `c133524a`); own waits, scopes and
     spawn-set phases only while the window is live; world-hub waits are never touched (they re-arm for the whole game;
     the game fires the ones the new facts make true on load, gate J2). The swap is written by `w3graph::synth::apply` of
     a walk that only consumes A's blocks, exits A's scopes, adds B's waits and switches the sets.

Then the window's own quests are exported on the rewritten template as before; that export verifies against the
rewritten template, the rewrite verified against the original.

### Validation [proven offline] (`rewriting_a_played_branch_gives_the_other_played_branch`, 6 directions)

References are synthesized (no donor) on T_spirit / T_keira / T_letho; the rewritten save is compared with the save a
human played for the other branch, on the delta's facts, journal keys and waits:

| pair (played A -> rewritten to B, compared with played B) | decision facts | journal entries | waits | graph swap |
| --- | --- | --- | --- | --- |
| spirit destroyed `91a5f1` -> released (`900fd9`) | 5 differ before, **0 after** | 3 / **0** | 24 / **0** | 21 waits + 7 scopes out, 3 in, 6 spawn sets |
| spirit released -> destroyed | 5 / **0** | 3 / **0** | 24 / **0** | reverse |
| Keira KM `94abd5` -> Radovid (`97cdc9`) | 2 / **0** | 4 / **0** | 2 / **0** | listener `f5290e77` -> `c133524a` |
| Keira Radovid -> KM | 2 / **0** | 4 / **0** | 2 / **0** | reverse |
| Letho invited `a4841f` -> elsewhere (`9d341b`) | 2 / **0** | 3 / **0** | 2 / **0** | thread `6850c728` + `7573f7fb` out |
| Letho elsewhere -> invited | 2 / **0** | 3 / **0** | 2 / **0** | thread + wait in (common aftermath live) |

The last row needed the common-aftermath rule: the elsewhere branch leaves nothing of its own live, but both branches'
`sq102_letho` waits (`974e8877`, `d8b0024d`, `e9e84a64`) still wait, so the window is live and B's waits are written.

### Full story [proven offline, not in game]

`w3al export --run <run> --template-dir C:\w3-fixtures\pack_501 --template-dir C:\w3-fixtures\session_V1` (scratch run
`chain2\runs\w9_story.toml`) and `full_story_on_the_latest_template`: template pack 54 chosen automatically; rewrites
W5 (facts -6 +1, journal -4 +3; Triss's / Yennefer's own waits long consumed: only facts and journal), W7 (facts -5 +6,
journal -6 +3, counters -4, the character-journal Emhyr listener `8d774fa2` and phase `db096e0e` out; persistent-content
equipment listeners left for the game; spawn sets not touched because the window is no longer live), W8 lodge (facts -1 +1,
journal -1 +1); then the W9 walks. VERIFIED, `check-save` clean, round trip PASS. Facts in the written save: no
`q309_triss_lover`, `sq202_yen_girlfriend`, no `q110_calmed_down_ciri`, `q110_visited_emhyr`, `q110_accepted_reward`, no
`q310_ciri_faced_lodge`, `q210_gave_lara_necklace`; pack-line facts kept (`q109_keira_to_km`, `q302_whoreson_dead`).
A second export, stop at Brothers in Arms on pack 43 with Keira to Radovid, Letho elsewhere, Junior spared, Folan dies,
Triss let go, Yennefer love (four windows / six quests rewritten, then Ugly Baby and the four invitations; scratch run
`chain2\runs\w6_keira.toml`) is VERIFIED too.

## 3. Which decisions can be rewritten on which template

Readers from `w3al template-scan` (fact conditions in the quest graphs outside the decision's own file, with their state
in the save: waiting / file running / file ran / not started; a file "ran" when the save holds a fact only that file
sets) and `decision-catalogue.md` section 2. Scene-file conditions are not scanned.

| decision | rewritable on | not on (fixed) | why | label |
| --- | --- | --- | --- | --- |
| D1 spirit | its window only (pack 8) | 13 and later | q107_swamps (Baron finale) branched on it: Anna, the Baron, the orphans | [proven: readers ran] |
| D3 Keira's date | - | 13 and later | no 5.01 save of pack 12 (no reference) | limit |
| D2 Keira KM / Radovid | 21, 33, 35, 42, 43 | 48 and later | q402 Keira phase / q403 battle read it (pack 43-47); before that only facts, journal and her listener | [proven offline on the pair; likely on later templates] |
| D2 Keira killed | 21-43 | 48+ | as above; her world state on later templates is "gone" in both branches | [hypothesis] |
| D4/D5 Letho invited / elsewhere | 21-43 | 48+ | the battle read it | [proven offline on the pair; likely later] |
| Letho abandoned | - | 21+ | walks only on T_letho, not a window of its own | limit |
| D6 Junior | 33-51 | 54 | Final Preparations (q310) read `q302_whoreson_dead` and despawned Dudu's phase | [likely] |
| D10 Folan / Vigi | 35-43 | 48+ | q403 (battle) read `q202_folan_lives` | [likely] |
| D11 King's Gambit side | its window (35) | 42+ | Cerys crowned (q206 / q208 played) | [likely] |
| D8 / D9 romance | 43, 48, 51, 54 | - | ending boards, q504 / q505 read the facts later; on 54, q310 already played its Triss/Yennefer dialogue variant with the pack answers (`6de355b4` / `48f5a8f1` choose the scene input) | [likely]; on 54 [hypothesis] for the q310 aftermath |
| D12 allies | its window (43) | 48+ | the battle | [proven by the line] |
| D13-D15 snowball / Emhyr / pay | 51, 54 | - | read by q502 (Ciri's fate), ending boards, epilogue (pending); the Vizima visit itself is not replayed | [likely] |
| D16 lodge | 54 | - | read by q502 only | [likely] |
| Dijkstra's leg (final_preparations.dijkstra) | its window (51) | 54 | Reason of State played with it | [likely] |
| D17 / D18 Reason of State | its window (51) | 54 | Radovid / Roche / Thaler deaths are world state (cg300_novigrad) | [likely] |

Unchanged quests: act-1 placeholders, `family_matters`, `a_towerful_of_mice` and `carnal_sins` (no 5.01 window
template yet: `[[line]]` entries; not recorded on later templates, so the save keeps the pack line's outcome and the plan
says so).

### Late completion: a later window's quest on an earlier template [proven offline, not in game] (2026-11-06)

A player who did Now or Never before sailing to Skellige (the pack line did it after: W5 = pack 42) used to lose it on
the Skellige templates (`beyond`). The registry's `late` list (w4_undvik, w4_kings_gambit: `late = ["now_or_never"]`)
moves such a quest to `Choice::late`; `chain::build_story` resolves it and replaces each option effect by its
`[effects.*.late]` table (`synth_effect::late_effects`): one `decision` auto-completion per quest, in order (sq301 *A Matter
of Life and Death*, whose end starts q309, then q309), with the player's answer forced (`pick` `d6a0ac74=geralt alone` /
`triss returned`, the sq301 kiss as a `scene_pick`) and ways the pack player did not take as `stop`s the policy routes
around (leaving Triss in the fight, `q309_triss_refused`; letting her go on alone, `q309_abandon_triss_rescue`). New in
`w3graph::complete::Options`: `picks`, `scene_picks`, `stops` (`w3al side-complete --pick --scene-pick --stop
--allow-decision --check PLAYED`). Edit mode offers the quest as `late` on a save sitting at such a template.

Evidence: `now_or_never_completed_late_on_the_skellige_template` (export on pack 33, go and stay, VERIFIED; decision facts
and both journal roots Success). Against pack 43 (stay, played much later): sq301 journal hit/miss/extra 25/4/0, facts
22/12/10 (misses: the pack's optional card games, fist fight, masks); q309 journal 21/0/2, facts 31/4/7 (fight-or-talk
variants with the fire guards). Against the W5 window walk (go, pack 42): q309 journal identical but 2 extra early
objectives Success, decision facts identical. Open: spawn sets the rescue chase switched on stay on (`q309_wh_rescue_chase*`;
pack 43 has none: the game drops them, the walk does not), never loaded in game.

## 4. Open limits

* Nothing of the rewrite has been loaded in game. First gate: the pack-54 full-story export (journal entries removed by
  us, Triss/Yennefer and Emhyr facts swapped late) and the pack-43 export (Keira's listener swapped to Radovid before the
  battle: does the battle play without Keira?).
* Later play that branched on the pack answer *inside* a rewritable decision's readers is not undone (romance on pack 54:
  the q310 dialogue variant; Emhyr: the Vizima visit is not in the world history, only in facts and journal).
* The world-hub split (`chain::is_hub_file`) is a file-name list; a decision whose live state sits in a hub file other
  than the character-journal listeners keeps the pack state there.
* `encodes` for `kings_gambit` (`in_game` = Cerys), `lord_of_undvik.vigi` (`play`) and `a_favor_for_a_friend` are not
  scanned (no option or no window template matches); the rest is.
* Combinations inside one window are measured with the other window answers held at the template's; interactions
  across windows (a rewritten W5 answer that a W6 walk would have read differently) are only right when the later
  window's own walk runs after the rewrite (true for the chosen template's own window, not for baked later windows).
* `template-scan` needs every window template and the donors of the W2 graph effects (`session_V1`, `session_V2`).

## 5. Edit mode (a second mode of the app, 2026-11-05 night)

Open any 5.01 save of the player's own and change it into a NEW save (`w3al edit-plan <sav>`, `w3al edit --save <sav> --set
quest.choice=option [--level N --crowns N --gear recommended --item NAME] --out-dir DIR`; app: Welcome mode switch, commands
`list_saves`, `save_thumbnail`, `save_brief`, `edit_plan`, `edit_apply`; code `crates/w3al-core/src/edit.rs`, `ui/edit.js`).
The opened file is only read and its bytes are compared again after writing.

* **Reading the save** [proven offline on the nine pack templates]: `Engine::load` builds every window's answer combinations on
  its own template (`chain::scan_window`, the builds `template-scan` uses; minutes the first time, cached in a JSON file keyed by
  questbook, registry, donor folders and template files). A window is *decided* when the save holds a fact the window writes and
  one combination is the best match on the facts that vary between combinations (mismatch limit max(3, a third of them); ties
  leave the choices they disagree on unknown), *not reached* when none of the window's facts is there, otherwise *unknown*.
  A save past a decided window cannot be before an earlier one (those become unknown). A save that IS a registered template
  (matched by file name) takes the registry's `encodes` over the facts where they differ (vigi, the Cerys trial, Keira's date are
  hand-set) and says so. Test `plan_reports_the_registry_answers`: 88 registry answers reproduced, 68 of them from the facts alone.
* **What can change**: a decided answer is rewritten (`chain::rewrite_all`) unless the `fixed` rules of the template the save sits
  at (the first window after the last decided one; the template itself when it is one) block it, or its window is not before that
  template, or the window template file is missing, or the answer is not known. The first window not reached is offered with its
  own synth effects when a dry build on the save succeeds (otherwise read-only with the build error); the rest of its quests take
  defaults. Level (1..=50), crowns and kit go through `start_kit` (new `items` field: single items of the recommended table).
* **Late quests** (2026-11-06): a not-reached quest that the save's template lists in `late` is offered as mode `late`
  (`w3al edit --set now_or_never.triss=go` on a w4_undvik save completes sq301 and q309 there). Window classification
  no longer counts a fact an earlier window's template already holds as evidence (`triss_dd_start` made a w4_undvik save
  read as past W5-W7).
* **Known limits**: an option that writes no deciding fact (`in_game`, `play`) can be mis-read on a save that is not a registered
  template; quests without a window template (`[[line]]`) are "not tracked"; nothing is loaded in game; the plan of an arbitrary
  save inherits the story chain's open limits (section 4).
