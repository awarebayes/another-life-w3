# Act 1 (White Orchard) content plan

Date 2026-11-05. Phase 3 of `W3_Another_Life___Plan.md`: "map White Orchard and the Witcher 2 import decisions to facts, write their TOML,
and capture the act templates. *Exit:* a save generated for 'stop after White Orchard' loads, and the import choices show up in game."
Builds on the gates of 2026-11-05 (G, P, S, F, R, M: `docs/research.md`) and the export path (`docs/findings/export-wiring.md`).
Quest ids below are journal file names seen in the lukwraith saves (`docs/research.md`, quest catalogue); English titles are only
labels for us and must be checked before they go into the UI.

## 0. Before starting (gates this plan depends on)

| Item | State | Why it matters |
| --- | --- | --- |
| Phase 2 exit test: `cargo tauri build` installer; pick a save, answer two dummy questions, export a file byte-identical to the template | **not run** (CLAUDE.md: scaffold only) | CLAUDE.md rule: no Phase N+1 work before Phase N's exit test. Run it first; it is short. |
| Quest effects in game (journal + facts + graph + community) | proven for 3 quests, 2 branches, a never-started quest, 2 donors, a remapped name | content can be captured, not guessed |
| Export from questbook | proven offline (G1, R1 byte-identical) | content TOML is the product path |

## 1. What act 1 contains (from the 45 lukwraith saves)

| Journal id | Kind | Started / finished (lukwraith pair) | Choice signals in the saves | Notes |
| --- | --- | --- | --- | --- |
| `questactprologue/q001beggining` | main | before 01 / 02->03 | `q001_nightmare_ended` | Kaer Morhen dream; world load to White Orchard |
| `questactprologue/q001yennsearch` | main | 02->03 / 37->38 | many `q002_*` at 37->38 | the act's spine; tracked quest of most saves |
| `questactprologue/q002griffin` | main | 31->32 / 36->37 | `q002_reward_accepted`, `q002_lashes_count`, inn brawl | the Beast of White Orchard |
| `questactprologue/q002healingwoman` | main (sub) | 32->33 / 32->33 | `q002_helped_herbalist`, `q002_know_about_lena` | starts and ends in one pair |
| `questactprologue/q002bandits` | main (sub) | 37->38 / 37->38 | `q002_inn_bandits_dead` | |
| `questactprologue/q002vizimapalace` | main | 37->38 / 41->42 | `q002_met_emhyr`, `q002_talked_to_yennefer`, `q002_var_attre_met` | Imperial Audience; act boundary (84 Novigrad scenes registered 41->42) |
| `minorquests/mq0001battlefieldmother` | side | 06->07 / 26->27 | `mq0001_*` (brother found, dog, shack) | |
| `minorquests/mq0002thefield` | side | 23->24 / 24->25 | `mq0002_merchant_ran`, `_stop_riding` (no completion fact) | **gate P** |
| `genericquests/mq0003freshwater` | side | 06->07 / 29->30 | `mq0003_bracelet_placed` / `_destroyed`, `_corpse_*`, `_noonwraith_*`, `mq0003_hamlet_inhabited` | likely a real branch (to verify with two donors) |
| `minorquests/mq0004lockedshed` | side | 22->23 / 22->23 | `mq0004_completed` | **gate F** |
| `minorquests/mq0005dwarfsmith` | side | 20->21 / 21->22 | brought vs spared (`24353285_2_0/_5_1` vs `_2_1`) | **gates G, S** (both branches captured) |
| `livingworld/lwprologuebandittreasure`, `lwprologuedeserterstash`, `lwprologuetemeriantreasure`, `tresurehuntings/th1001viper` | activities | various | none named | no story choice; v1 = completed/skipped only, low priority |
| Witcher 2 import | facts | `import_council_exists`, `import_geralt_rescued_triss`, `import_letho_alive`, `import_sheala_alive` appear at **37->38** (all = 1 in lukwraith) | the plan's Phase 3 exit test | where they are asked and which values exist: research task R1 |

## 2. Template strategy for act 1

The gates showed: a quest counts as done when its delta (journal + facts + graph + community) comes from a donor; a skipped quest must be
**never started** in the template (journal entries cannot be removed); deltas from several donors compose on one template.

**Recommended: one clean act-1 template plus one donor per quest branch, all from the same playthrough.**
- **T1 (template):** a new game played through the act-1 MAIN story only, skipping every side quest and activity (do not even talk to their
  quest givers), saved right after the griffin is dealt with and before leaving White Orchard (the last point where side quests are still open).
  Every side quest is then "never started" in T1, which is what F proved works for a completion.
- **Donors:** from T1, for each side quest and each branch: load T1, play only that quest to the end with that choice, save. Every donor's name
  table extends T1's (prefix mode, no remap needed), every guard matches T1 by construction, and two donors never edit the same scope (each
  quest has its own scopes; the catalogue lists the shared "listener" scopes to watch, `docs/research.md`).
- **"Stop after White Orchard" save:** T1 + chosen deltas, then the act boundary (Imperial Audience) as a second template T2 captured from the
  same run after `q002vizimapalace` (or the plan's "free-roam spawn nearest the stop point"; open decision 3).
- **Why not lukwraith's saves:** they are third-party (cannot be shipped), and their side quests are already done (cannot be "skipped").
  They stay our research fixtures.
- **What we ship:** templates and donors are made by us from our own playthrough (plan: "template saves ... made by contributors from their
  own playthroughs"); release asset, not repo. Alternative to shipping donors: embed each delta's subtree bytes in the TOML (engineering task E3).

## 3. Capture sessions for you (the player), in order

All on 5.01, no mods, one playthrough, saves copied to `C:\w3-fixtures\act1\` by me; you play, I diff.
1. **Session A (longest, ~1.5-2.5 h):** new game -> Kaer Morhen -> White Orchard main story only -> griffin done -> stop. Make a manual save at
   each main beat (I tell you which), and the final one = **T1**. Skip all side quests, treasure, notice boards.
2. **Session B (short pieces, ~10-30 min each):** from T1, one quest per piece, one save at the end of each:
   mq0005 brought, mq0005 spared, mq0004, mq0002, mq0001, mq0003 (each branch we find; R2 says which), q002healingwoman if it is skippable.
   Also a "started" save per quest (accept it, save) if we decide to offer "started" (open decision 4).
3. **Session C:** from T1 continue the main story to the Imperial Audience with each W2-import answer set R1 finds (the import facts appear
   there in lukwraith's run), saving after the conversation: donors for the import choices; then the act boundary save **T2**.
4. **Exit test session:** the exported "stop after White Orchard" saves (a few answer combinations): load, check journal, talk to one NPC per
   choice, start the next quest; results logged in `docs/playtests.md`.

## 4. Work that runs in parallel (agents / me)

- **R1 W2 import research (read-only):** grep the plain-text scripts (`content\content0\scripts`) for `import_` facts: where they are set,
  which values, which later scripts read them (that tells us where they "show up in game" for the exit test). One-line quotes only.
- **R2 Act-1 quest choices research:** list each act-1 quest's real decisions and outcomes from research sources (wiki text is CC BY-SA:
  research only, original summaries); map each to the fact/journal signals in the saves; mark what needs a second donor to confirm.
- **E1 Phase 2 exit test** (installer, dummy export byte-identical) and **E2 Tauri UI with real effects** (welcome: pick template + donors
  folder; export through the app; first end-to-end with the G1 test questbook).
- **E3 Self-contained deltas:** store copied subtree bytes (and the donor's names) in the TOML so export needs no donor files.
- **C1 Content TOML:** one file per quest in `questbook/01_white_orchard/` (replacing the two placeholders), effects only from captured
  deltas (`journal-diff` + `facts --diff` + `graph-diff --toml` with scopes), original second-person narration, levels, order, requires.
- **C2 Capture tooling:** a `w3al capture T D --quest mq0005` that runs the three diffs, scopes the graph by the quest's own scopes and
  drops the known toggling scopes, and prints one ready effect block (today these are three commands and hand selection).

## 5. Decisions needed from you

1. Play the clean template run (Session A) yourself, or look for an existing clean save of your own?
2. Act-1 quest scope for v1: main + the five minor quests (mq0001-mq0005) + W2 import; activities (treasures) as completed/skipped only, or left out?
3. Stop point after White Orchard: the Vizima palace (needs T2, risk of a stuck Geralt per the plan) or White Orchard free roam?
4. Offer "started" for side quests in v1, or only completed / skipped?

## 6. Exit criteria for Phase 3 (from the plan, made concrete)
- `questbook/01_white_orchard/*.toml` has no `placeholder = true`; `w3al lint` 0 errors; every effect has a `source` naming the save pair.
- For at least three answer combinations (all done / mixed branches / all side quests skipped) export writes a save that loads, shows the
  journal as answered, and a choice-dependent NPC behaves accordingly (as in gates G/S).
- The W2 import answers are present as facts and show up where R1 says they are read (at the latest in Kaer Morhen, checked later).

## 7. Decisions (2026-11-05, the user)

- **Witcher 2 import questions are NOT in v1.** The template keeps its own `import_*` facts (lukwraith 38: all four = 1), as when a player
  does not simulate a W2 save. The Phase 3 exit test becomes: a save for "stop after White Orchard" loads, the journal is right, and the
  game plays on from it.
- **No clean act-1 template is needed for v1:** act 1 offers no choices (White Orchard minor quests do not change endings, allies or
  survival, the plan's v1 coverage rule), so a complete end-of-White-Orchard save serves (research: WestAard, lukwraith 37; release: one
  of our own). Section 2/3 (clean template, donors per side quest) is kept as the method for later, not as v1 work.
- Effort moves to act 2 (Velen / Novigrad), where choices change endings; first step: a catalogue of act-2 decisions mapped to the saves we
  have (`C:\w3-backup\user_saves_stash`, `C:\w3-extract`) and to what must still be captured.
