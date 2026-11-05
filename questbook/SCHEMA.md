# Questbook schema

One TOML file per quest, anywhere under `questbook/` (folders are just for humans:
`01_white_orchard/`, `02_velen/`, ...). Loaded by `w3al-core`; checked by `w3al lint`.
Unknown keys are rejected, so typos fail loudly.

The questbook is licensed **CC BY-SA 4.0** (the code is MIT/Apache-2.0). Write original
narration; never paste game text or wiki text. Every quest must be traceable.

## Quest

| key | type | required | meaning |
| --- | --- | --- | --- |
| `id` | string | yes | Unique, `snake_case`. Referenced by `requires`, `excludes` and `run.toml`. |
| `title` | string | yes | Shown on the quest page. |
| `act` | int | yes | 1..6, matches the act templates. |
| `order` | int | yes | Global chronological sort key; **unique**. Leave gaps (1010, 1020, ...). |
| `kind` | `main` \| `secondary` \| `contract` | yes | Skipping a `main` quest locks every later `main` quest. |
| `region` | `white_orchard` \| `velen` \| `novigrad` \| `skellige` \| `kaer_morhen` \| `vizima` | yes | Tag on the quest page; filter on the review screen. |
| `level` | int | yes | The game's recommended level; tag on the quest page. A side quest more than 5 levels above the nearest main quest gets a soft warning. |
| `requires` | list of ids | no | Quests that must be completed/started first. Must have a smaller `order`; must be acyclic. |
| `excludes` | list of ids | no | If any of these was completed, this quest is not applied. |
| `image` | string | no | Relative path; decoration only. See licensing below. |
| `summary` | string | yes | Short, second-person, journal-style, original. |
| `started_effects` | effects key | no | Applied when the player leaves the quest *started*. |
| `done_effects` | effects key | no | Applied when *completed*, after the chosen option effects. |

### Status semantics

* **completed**: the player's answer to every `[[choices]]` (or its `default`) is applied,
  then `done_effects`.
* **started**: only `started_effects`. Whether this is stable in game is an open question
  (docs/research.md), so treat it as experimental.
* **skipped** (or absent from the run): nothing is applied.

A quest with **no** `[[choices]]` offers only completed / started / skipped.

## Choices

```toml
[[choices]]
id      = "botchling"            # unique within the quest
prompt  = "What do you do?"
default = "lubberkin"            # REQUIRED: the option used when the player doesn't care
options = [
  { id = "lubberkin", text = "Turn it into a lubberkin", effects = "botchling_saved" },
  { id = "kill",      text = "Kill it",                  effects = "botchling_killed" },
]
```

At least two options. `effects` is optional (an option may change nothing) and names a
key in the quest's `[effects.*]` tables.

## Effects

```toml
[effects.botchling_saved]
source      = "scripts/.../q103.ws:412"   # REQUIRED: script line, quest graph or save diff
placeholder = false                       # true = scaffold value, lint warns
facts       = { "real_fact_name" = 1 }    # fact identifier -> value
facts_remove = ["fact_name"]                 # removed after `facts` (FactsRemove); for facts the donor no longer has
# [effects.botchling_saved.graph] / [effects.botchling_saved.synth]: quest graph from a donor, or walked (below)
quests      = { "real_quest_id" = "completed" }   # started | completed | failed
journal     = ["real_journal_entry"]
```

### Journal effects (quest log)

A quest's quest-log state (`CJournalManager`) is addressed by GUID paths that cannot be derived from names,
so these tables are **captured, never typed**: `w3al journal-diff before.sav after.sav --grep <quest> --key <effects key>`
prints a ready-to-paste block with `source` = the two save names. Design and evidence:
`docs/findings/journal-editor-design.md`. Lint validates GUIDs (32 hex digits), status names and path depth.
Export applies them. The journal alone does NOT make a quest count as done in play (gate J: the stale quest
graph re-ran the quest); give the same effect a `graph` table too (below).

```toml
[[effects.done.journal_status]]          # an entry the template already has
label = "minorquests/mq0005dwarfsmith"   # readable path, reviewers only
path  = ["67e5afe3ceb5c148a9cde5c4a99c7a84", "0f3305c02a250d4aa75c6fba97048763"]
from  = "JS_Active"                      # optional guard: the template must be this (or already `to`)
to    = "JS_Success"                     # JS_Inactive | JS_Active | JS_Success | JS_Failed
# unread = true                          # optional

[[effects.done.journal_append]]          # make sure the entry exists (appended at the end if missing)
label  = "minorquests/mq0005dwarfsmith/f02a0417/753019eb"
status = "JS_Success"
unread = true
steps  = [                               # 1..5 steps (abbreviated here); flags are derived (0 inner, 1 last)
  { guid = "67e5afe3ceb5c148a9cde5c4a99c7a84", resource = 'gameplay\journal\quests\minorquests.journal' },
  { guid = "753019eba7b3cf48ae3f0b4af472783c", resource = "" },
]

[[effects.done.journal_counters]]        # set or add an objective counter
guid  = "753019eba7b3cf48ae3f0b4af472783c"
count = 0
# at  = 27                               # optional insert position, only valid on the capture's own 'before' save

[effects.done.journal_track]             # tracked quest / highlighted objective afterwards (both optional)
quest     = "7c198a768adc3046b34b5a730b5b53c9"
objective = "6b9fcac1d1bc9946990bdc429ea8978c"
```

`journal = ["..."]` (a list of ids) is the old scaffold form; it is still refused, like `quests = {...}`.

### Graph effects (quest graph and spawn sets)

What the game runs next lives in the quest graph (`questSystem`) and the NPC spawn sets (`community`). A quest
counts as done in play when its graph scopes, listener blocks, scene-list entries and spawn set / stub are
transplanted from a **donor** save that played past it, together with its journal and facts (gates G, P, S, F, R,
2026-11-05; `docs/findings/quest-graph-design.md`). Captured, never typed: `w3al graph-diff before.sav donor.sav
--block questSystem --toml <effects key> --scope <scope> ...` prints the table (rename the key); community ops
(remove the spawn set, swap-remove its stub, recount) are added by hand from `w3al tree <save> --path ...`.

```toml
[effects.done.graph]
source = "save diff ManualSave_fd3e1_... -> ManualSave_8559a_... (w3al graph-diff)"   # optional
donor  = "ManualSave_8559a_7ea4d800_4038706"   # stem of the donor save; required when an op copies from it
ops = [
  { op = "remove",      path = "questSystem/quest[witcher3_quest.w2quest]/.../questBlock[62a65bce]", before = "#d5942b6b" },
  { op = "insert",      path = "questSystem/.../questThread[7fd9eeae]", after = "#20d584ac" },
  { op = "replace",     path = "questSystem/.../ExternalDialog[mq0005_arsonist]", before = "#91b604b6", after = "#07621930" },
  { op = "recount",     path = "questSystem/.../questThread/numBlocks", of = "questBlock" },
  { op = "swap-remove", path = "community/stubs/a#8", before = "#dda3a33e" },
  { op = "recount",     path = "community/stubs/numStubs", of = "a" },
]
```

* `op`: `replace` (template node := donor node), `remove`, `swap-remove` (the last sibling of the same name takes
  the slot; what the game does to stubs), `insert` (donor node, after its donor predecessor), `recount` (count var :=
  number of `of` children; only the pairs of `graph::COUNT_RULES`: `questThread/numBlocks` - `questBlock`,
  `questThread/numThreads` - `questThread`, `CQuestExternalScenePlayer/tagsCount` - `ExternalDialog`,
  `communities/numCommunities` - `c`, `stubs/numStubs` - `a`).
* `path`: as `graph-diff` prints it; must start with `questSystem/` or `community/` (the blocks proven in game).
* Guards: `before` = hash of the template's node (replace / remove / swap-remove), `after` = hash of the donor's
  node (replace / insert), `#` + 8 lowercase hex digits; `""` = not checked. Export refuses a save or donor whose
  node does not match (wrong template, wrong donor, or an earlier effect moved the node).
* `donor`: a save stem (no extension, no folders). Donor saves are real saves and never part of the questbook;
  export looks for `<donor>.sav` in its donor folders (`donors = [...]` in the run file, or `w3al export --donors DIR`).
  A delta that only removes and recounts copies nothing and needs no donor file.
* Composition: two effects may not touch the same node (or one a node under the other's) unless both come from
  the same donor; shared lists (scene list, spawn sets, stubs) take point ops plus `recount`, never a whole-list
  `replace`. A second donor whose name table does not extend the save's needs `allow_name_remap = true` in the run
  file (HYPOTHESIS: proven offline only).
* Per effect, export applies `graph` (or `synth`), then the journal tables, then `facts`, then `facts_remove`; effects in run order.

### Synth effects (built from the game's quest graphs, no donor)

Instead of a captured `graph` delta from a donor save, an effect can name the decision and let export walk it
through the quest graphs of the player's own install (read-only), exactly as `w3al synth-apply` does
(`docs/plan/no-human-pipeline.md`). The walk writes the quest frontier (`questSystem`: fired and ended blocks out,
new waits and scopes in, scene registry), the journal and the facts itself.

```toml
[effects.keira_km]
source = "quest graph q109_wrapup (w3al synth-walk on T_keira; checked against donor 94abd5)"

[effects.keira_km.synth]
start   = ["215f2719"]               # blocks waiting in the save to fire: GUID (8 or 32 hex) [":input"]
through = true                       # single-output waits count as done (the player did them)
pick    = ["d24f649d=Keira to KM"]   # decision block = output socket to take
stop    = ["4c2a9b2d"]               # waits to park at (the end of the decision window)
scene_pick = []                      # "GUID=section": the choice line inside that block's scene file, for decisions
                                     # the quest graph does not see (Letho: "ed477b93=sq102_go _to_km_letho")
```

* Find the keys with `w3al synth-walk --show-frontier` (the template's waits) and check the walk with
  `w3al synth-walk ... --donor <played save>` before writing the table; same arguments, same result. Section names for
  `scene_pick` come from `w3al scene-info --install <game> <scene file>` (the walk's `MAYBE` lines name the file).
* Lint: `start` plus `pick` or `stop` (or both); keys are 8 or 32 hex digits, picks `GUID=socket`, scene picks
  `GUID=section`; not together with `graph` on the same effect; never `placeholder = true`. Whether the blocks exist and
  the socket is real is checked at export against the graphs.
* Export refuses the effect when a `start` block is not waiting in the save at that point (wrong template, or an
  earlier effect moved past it), when the walk has an unresolved point (a multi-output wait without a pick, a condition
  it cannot evaluate), or when the game folder is missing. The game folder is `game_dir` in the run file
  (`w3al export --game-dir DIR`, the app's Game folder field), default the Steam folder
  `C:\Program Files (x86)\Steam\steamapps\common\The Witcher 3`. The graphs are loaded once per export.
* The effect's own journal tables, `facts` and `facts_remove` apply after the walk (for what the graph cannot give:
  script side effects such as `q105_new_heart_killed`).
* The walk also writes the spawn sets (`community`: phase switches, new sets from the `.w2comm` files, deactivated sets
  with their stubs and `entityData`), the facts set inside scene files (`.w2scene`, with `scene_pick` where a choice line
  decides) and the objective counters of new objectives (`.journal`). Not written: script side effects, rewards, map pins,
  the stubs of newly active sets (the game creates them on load: HYPOTHESIS until gate K3).
  `docs/plan/no-human-pipeline.md` section 2.
* A window whose template save does not exist is reached by chaining: the quest before it carries the preparatory walk in its
  `done_effects` (a synth table with the pack line's picks and the stops that mark the window's start), and the window's quest
  names it in `requires`. Export applies effects in quest order, so the walk runs first on the real template and the window's
  choices start from its result; `w3al synth-apply` with the same arguments writes that intermediate save as a file for checks.
  Example: `05_kaer_morhen/ugly_baby.toml` (pack 43 -> the "gathering allies" phase) and `brothers_in_arms.toml`. A run that
  marks the preparatory quest `started` or `skipped` makes the window's effects refuse (their start blocks are not waiting).

#### Auto-complete mode (`complete`)

```toml
[effects.done.synth]
complete = "67e5afe3ceb5c148a9cde5c4a99c7a84/8ce50f9eee92164cade1edae10fe019b"   # the quest's journal root GROUP/QUEST
```

Instead of `start` / `pick` / `stop`, the walker finishes the whole quest by itself from whatever state the save holds
it in (`crates/w3graph/src/complete.rs`): not discovered (its phase runs, its journal root is absent), active, or not
started (a hub wait upstream of its phase is fired on the way to it), or suspended because the save is in another world
(walked from its entry; its suspended scope leaves the save). Policy, deterministic: starts = the quest's own waits that
lead to its Success, the furthest first; inside the quest every wait with an output towards the goal counts as done, the
output nearest the goal is taken where there are several (a *pick*, listed by `w3al side-complete`), a wait with no
output towards the goal (failure listeners, despawns) stays waiting; a condition the walker cannot evaluate or a random
block takes the output nearest the goal, else its first; branches that meet through facts count as connected; the
quest's phase ends only after every start was walked; the goal is the journal root's Success, then the phase's end;
edges into the root's Failure are never taken. Outside the quest nothing is walked `through` (the hub runs to its next
wait; the save's own fact listeners fire, and one whose run meets a point the walker cannot decide is left for the game).

* Refused (export says why): the root is already Success or Failed; the quest's graph holds a decision-catalogue file
  (`w3graph::complete::DECISION_FILES`, docs/plan/decision-catalogue.md); its phase is not running and no wait leads to
  it; no route to Success; an unresolved point; the phase still runs after Success with a wait that could end it.
* Lint: `complete` is `GROUP/QUEST` or `QUEST` (32 hex digits each) and stands alone (no `start` / `pick` / `stop` /
  `scene_pick`). Check a quest with `w3al side-complete --install <game> --template <save> <key>`.

#### Late completion (`[effects.*.late]`)

```toml
[effects.triss_leaves.late]          # the same answer, completed on a LATER template that has not reached the quest
complete   = ["<sq301 GROUP/QUEST>", "<q309 GROUP/QUEST>"]   # auto-completed in order (a prerequisite first)
pick       = ["d6a0ac74=geralt alone"]                        # the player's answer: forced outputs
scene_pick = ["ebdeb52d=sq301_00a_ok"]                        # forced dialogue lines
stop       = ["f9fc4999"]                                     # waits the player did not take (the policy routes around)
```

A decision quest whose window template lies later on the line than where the player did it (Now or Never before
Skellige) is completed late on the earlier template when that template lists it in its registry `late` list. Each quest
of `complete` runs the auto-complete walk above with the decision-catalogue refusal lifted and the forced answers taken;
the effect's own `facts` / journal tables apply after the last quest; `synth` / `graph` (the window's own walk) are not
used. Every option effect of a `late` quest needs this table (lint). Try the walk first with `w3al side-complete --install
<game> --template <save> --allow-decision --pick ... --scene-pick ... --stop ... --check <played save> KEY...`.

## Side quests (`questbook/side/`)

Generated, one file per quest without a consequence the player chooses (side quests, contracts, treasure and scavenger
hunts, Gwent, races, fist fights; main-story quests between decisions under `side/main/`), by
`w3al gen-side-quests --install <game> --out questbook/side --template-dir <templates>`. Not quests of the decision
questbook (loaded apart: `Questbook::side`); the run marks them `side_quests = { <id> = "completed" | "skipped" }` and
export applies the completed ones after the story's own effects, on the chosen template.

```toml
id = "mq1001wolvesanddogs"                 # the journal file stem, snake_case
kind = "side"                               # side | contract | treasure | scavenger | gwent | race | fistfight | main
region = "velen"                            # white_orchard | velen | novigrad | skellige | kaer_morhen | vizima | other
act = 2                                     # 1..6 as the questbook's acts
order = 21001                               # unique among side quests (act, kind, journal file)
title_id = 352169                           # string id of the official title, read from the player's install at run time
fallback_title = "quests/minor_quests/no_mans_land/mq1001_wolves_and_dogs"   # internal graph path, when no install
journal = "67e5afe3ceb5c148a9cde5c4a99c7a84/8ce50f9eee92164cade1edae10fe019b" # journal root GROUP/QUEST
phase = "quests/minor_quests/no_mans_land/mq1001_wolves_and_dogs.w2phase"   # the quest's graph file
done_effects = "done"
auto_picks = ["8d9e369d....=out hanna missing"]   # optional: see "Consequence check" below (block GUID=socket the walk takes)
[templates]                                 # the generator's trial per window template: ok | done | refusal
w4_undvik = "ok"
[effects.done]
source = "quest graph ... (auto-complete walk ...)"
[effects.done.synth]
complete = "67e5afe3ceb5c148a9cde5c4a99c7a84/8ce50f9eee92164cade1edae10fe019b"
```

* No game text: titles come from the install (`title_id`); ids, GUIDs and graph paths are identifiers.
* Lint: unique ids (also against the decision questbook) and orders, one file per journal root, region and act in
  range, the done effect present with a `complete` naming the same root, never a placeholder.
* The generator skips the expansions (out of v1), demo / test quests and the quests whose graph the decision questbook
  already walks, and refuses decision quests; a quest is written when the walk finishes it on at least one registered
  window template.

### Consequence check (`w3graph::consequence`)

Only quests *without consequences* are marked done automatically. The generator and the `complete` walk (export,
`side-complete`) refuse a quest with the reason "has a decision with consequences" when its walk passes a player
decision, i.e. a dialogue block (scene or interaction dialog) of the quest's own region with two or more connected
output sockets, and the branches behind those sockets differ in something the game reads elsewhere:

* a fact set on one branch only (by a fact block, or by the scene on the way to that output) is read outside the
  quest: by a pause or condition block of a graph file that is not one of the quest's files, by a flow condition of a
  scene the quest does not play, by a script (`FactsQuerySum("id")`, `FactsQuery`, `FactsDoesExist`) or by a spawn set
  the quest does not switch (a name mention); or
* the branches end with different journal outcomes that matter: an entry that is Failed on one branch and not on
  another, or any difference in the quest's root entry; or
* an in-scene choice line (inside the `.w2scene`, which the walker does not pick) sets a fact read outside the quest.

Trivial decisions stay allowed: cosmetic dialogue, routes that converge, differences only the quest itself reads.
Such a decision whose branches still differ in facts or alternative Success entries is listed in `auto_picks` as
`block GUID=socket` (the form `w3al side-complete --pick` takes), so the UI can show what the walk chose for the
player. Not considered: rewards, spawn-set differences between branches, facts a script builds from strings,
conditions other than facts. Evidence for a fact: `w3al fact-readers --install <game> FACT...`. A quest completed
with the player's own answers (`decision` completions, `side-complete --allow-decision`) skips the check.

## Effect rules (all quests)

* **Never guess a fact name.** If it can't be traced to a script, a quest graph or a diff
  between two real saves, leave it out and add an open question to `docs/research.md`.
* Scaffold values must say `placeholder = true`, a `PLACEHOLDER_` name prefix and a
  `# PLACEHOLDER` comment. `w3al lint` reports each one as a warning so none ships by accident.
* `[effects.*]` tables must be referenced by something (lint warns on unused ones).

## Template registry (`questbook/templates.toml`)

Not a quest file (the loader skips it; `w3al lint` checks it). It names the window templates the story chain chooses
from (design: `docs/plan/story-chain.md`). The saves themselves are never in the repo: export looks for `<stem>.sav` in
the template folders (run file `template_dirs = [...]`, the app's Templates folder, `w3al export --template-dir`;
default: a `templates` folder next to the questbook folder). A run with `template = "..."` uses that save instead
(manual override, no rewriting).

```toml
[[template]]
id     = "w9_skellige_act3"
stem   = "ManualSave_4dcef_7ea51000_400b253"   # file stem, no folders
line   = 54                                     # position on the template line (pack save number)
quests = ["child_of_the_elder_blood"]           # the window: quests whose effects apply on this save
source = "pack 54, 5.01 re-save"
late   = []                                     # quests of LATER windows this save completes late ([effects.*.late])
[template.encodes]                              # earlier quests this save already holds, with the answers
get_junior = { junior = "kill" }
ugly_baby  = {}                                 # done, no choices
[template.fixed]                                # earlier answers that cannot be changed on this save
get_junior = "Final Preparations already read q302_whoreson_dead"
"final_preparations.dijkstra" = "Reason of State played with it"     # one choice
"kings_gambit.side=refuse" = "..."                                    # one target option

[[line]]                                        # a quest on the line without a usable window template
quest = "a_favor_for_a_friend"
line  = 12
note  = "pack 12 has no 5.01 re-save"
```

* `encodes` is **derived, never typed**: `w3al template-scan --template-dir ... --donors ... --allow-name-remap` exports
  every answer combination of each earlier window on that window's own template and prints the best match against each
  later template (by the facts that vary between the combinations), with the readers of those facts and their state in
  the save (evidence for `fixed`). A hand-set value (an answer the scan cannot express) carries a comment saying so.
* Lint: unknown ids, quests or options; a quest in two windows; `encodes` naming a quest not earlier on the line; bad
  `fixed` keys; a questbook quest without a place on the line (warning).
* Choice: the latest template at or before the run's stop quest (by `line`) whose `fixed` entries do not block one of the
  player's differing answers and whose file is found. Earlier answers that differ and are not `fixed` are rewritten after
  the fact (facts, journal, live quest-graph state; measured on their own window template, so that template's file is
  needed too). `w3al chain-plan --run R` prints the choice.
* `late`: a quest of a later window that the run completes is normally lost on an earlier template (`beyond`); listed in
  `late`, it is completed there with the run's answer from its `[effects.*.late]` tables, before the window's own quests
  (edit mode offers it as `late` on a save sitting at that template). Lint: a known quest, later on the line, with a
  `late` table on every option effect.

## Licensing of images

Never commit CD PROJEKT RED art or wiki images. Order of preference: images the player
provides locally, contributor-made screenshots under CDPR's fan content terms (read the
current terms first), then original art. The UI works fully without any image.

## Checklist for a new quest

1. Copy a sample, set `id`, `order`, `kind`, `region`, `level`.
2. Write an original `summary`.
3. Add `[[choices]]` only for decisions that change later state.
4. Fill `[effects.*]` with sourced names, or leave `placeholder = true`.
5. `cargo run -p w3al-cli -- lint` must report 0 errors.
