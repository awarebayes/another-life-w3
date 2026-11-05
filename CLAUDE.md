# Another Life for The Witcher 3

Open-source Rust tool: the player walks Witcher 3's story as an illustrated text RPG, then
we write one `.sav` (Remastered PC, patch 5.01) whose world state matches their choices.
Plan: `W3_Another_Life___Plan.md` (wins over this file on scope). Unofficial, not affiliated
with CD PROJEKT RED.

## Phase status

| Phase | State |
| --- | --- |
| 0 Save-format spike (GATE) | **Passed 2026-11-04 (facts only; scope below).** Container, footer, variable table, `RB` (u16 NM name index + u32 offset of each top-level `BS`), the `NM`/`MANU` name table, the typed token tree (parse -> serialize and recomputed table equal on 101 files), the facts blob, `rebase` (no-op byte-exact on 118 files, growth/shrink checked on 15 saves) and fact `apply` are built and tested offline. In game (user-reported + our re-read, 2026-11-04): an unedited repack, gate A (new fact, facts block +36 bytes, offsets rebased) and gate B (exact fact restore) all load, play, and the game's own re-saves still contain our edits (`w3al_gate_a_test_fact` survived; `q001_gwint_game_won` kept one record). **Visible effect shown (user-reported):** `gwent_difficulty` set to 3 loads as Hard, the same fact removed loads as Normal (the game's own default), original reads Easy. **Scope of the pass:** facts only, on the user's own 5.01 save and one lukwraith save. NOT proven: quest/journal edits (`questSystem`, `CJournalManager`), position/world, DLC/mod state, other patches. Findings in `docs/research.md`. |
| 1 `w3save` crate | Container, footer/variable-table/RB readers, packed strings, typed tree, `facts` (parse, set, remove, exact restore), `Document::rebase`, `Document::apply` for facts are implemented but unproven in game (hypotheses marked `HYPOTHESIS:`). `Document::apply` still covers facts only; journal and graph edits go through `journal::apply` and `graph::transplant_with` (export and `journal-set`); `position::*` is `todo!()`. `journal` module (read, diff, edit; reproduces the game's own journal bytes for lukwraith 21 -> 22) is usable through `w3al journal-set` only; in-game gate J (`C:\w3-out\gate_J`) pending, design in `docs/findings/journal-editor-design.md`. Gate J done 2026-11-04: journal shows the quest done, but the stale quest graph reopens it. `graph` module (keyed diff + transplant of `questSystem`/`community`, byte-exact replay on 86 of 86 pairs) via `w3al graph-diff` and `journal-set --graph-*`; in-game gates G, P, S, F passed 2026-11-05 (graph + community transplant makes a quest count as done in play: 3 quests, 2 branches, a never-started quest; R = two donors on one template passed too; X1 = two act-2 decisions on one template and K2 = a branch synthesized from the game's quest graph without play passed the same day), design in `docs/findings/quest-graph-design.md`; name remap for other donors (gate M passed for a remapped CName; other value types offline only) in `docs/findings/name-remap.md`. |
| 2 Tauri shell | Scaffold: six commands + `ui/` wired; works against the sample questbook. Export (2026-11-05) applies graph, journal and fact effects from the questbook (`[effects.*.graph]`, donor saves found by stem in `donors` folders, `--allow-name-remap` opt-in) and reproduces gate saves G1 and R1 byte for byte (`crates/w3al-core/tests/export_fixtures.rs`, `docs/findings/export-wiring.md`). It still refuses placeholder effects (the whole sample questbook is placeholder) and the legacy `quests`/`journal` id lists. |
| 3 Research + act 1 content | Not started. Sample quests use `PLACEHOLDER_` names. |
| 4 Remaining content + Review | Review screen exists; content not started. |
| 5 Release | Not started. |

Do not start Phase N+1 work until Phase N's exit test in the plan passes.

## Layout

- `crates/w3save` - container, payload map, regions, facts, packed strings. All game-format knowledge lives here.
- `crates/w3al-core` - questbook (TOML), validation, run state, answers -> effects, export. Never touches bytes.
- `crates/w3al-cli` - `w3al dump | roundtrip | diff | facts | lint | check-save | export [--stem] | edit-fact | journal | journal-diff | journal-set | tree | graph-diff | graph-lookup | synth-walk | synth-apply | questbook-json | edit-plan | edit | character | character-set | side-complete | fact-readers`. Start kit: level, crowns, recommended gear, `--skill-points`, `--no-ngplus` (all shipped templates are New Game+; docs/findings/new-game-plus.md). Late completion: a window quest the template has not reached is walked late with the player's answer (`[effects.*.late]`, registry `late`). Side quests whose completing branches differ in outward-read facts or failed objectives are refused (`w3graph::consequence`). Edit mode (`w3al_core::edit`, `ui/edit.js`): open one of the player's own saves read-only, read its answers from the facts, change what chain.rs can rewrite, write a NEW save (docs/plan/story-chain.md section 5).
- `crates/w3graph` - reads the quest graphs of the player's own install (bundles, CR2W) at run time and walks a decision through them: donor-free synthesis (`docs/plan/no-human-pipeline.md`). No game data in the repo; fixture tests need `W3AL_GRAPHS` (extracted folder) or `W3AL_INSTALL`.
- `src-tauri/` - Tauri 2 shell (package `w3al-app`), six `invoke` commands.
- `ui/` - static HTML/CSS/JS, no build step, no Node. `withGlobalTauri` exposes `window.__TAURI__`.
- `questbook/` - quest TOML (CC BY-SA). Schema in `questbook/SCHEMA.md`.
- `docs/research.md` - open questions and findings. `DECISIONS.md` - scaffold assumptions.

## Commands

```bash
cargo build                       # w3save, w3al-core, w3al-cli (Tauri shell is excluded from default members)
cargo test                        # same set
cargo run -p w3al-cli -- lint     # validate questbook/ (must be 0 errors)
cargo run -p w3al-cli -- roundtrip "$W3AL_FIXTURES/some.sav"
cargo tauri dev                   # run the app (needs `cargo install tauri-cli --version "^2" --locked`)
cargo tauri build                 # NSIS installer on Windows
```

UI preview without Tauri: `w3al questbook-json > ui/dev-questbook.json`, serve `ui/` with any
static server. Only the story pages work; picking saves and export need the desktop app.

## Fixtures

Real saves are large and personal and are never committed (`*.sav` is gitignored). Point
`W3AL_FIXTURES` at a folder of **copies**. Never run experiments on files in the game's own
`gamesaves` folder.

## Rules

1. **Never guess fact names or offsets.** Trace each to a script line, a quest graph or a diff
   between two real saves; otherwise write an open question in `docs/research.md` and mark the
   TOML effect `placeholder = true` with a `PLACEHOLDER_` name.
2. **Never overwrite an existing save.** Files are opened with `create_new`; the output is a
   new `ManualSave_<a>_<b>_<c>.sav` (the Load list shows no quest name for other names) plus sidecars.
3. **Never skip an exit test.** Phase 0 is a hard go/no-go.
4. Round trip means decompress -> parse -> serialize -> compare **decoded payloads**. Compressed
   bytes will not match the game's (different LZ4 encoder) and that is fine.
5. `w3save` code that depends on an unconfirmed format detail carries a `HYPOTHESIS:` comment;
   code that cannot be written without evidence is `todo!()` or returns `Error::Unverified`.
6. Reference projects are for understanding. Port only MIT code, with attribution (mustiny8402
   recovery tool, Atvaark/W3SavegameEditor). Do not copy unlicensed repos; do not decompile the
   closed Nexus editor.
7. No CDPR files, wiki text or wiki images stored in the repo. Narration is original, second person.
   Images may be shown by linking to their public URLs (e.g. the Witcher Wiki on Fandom, `static.wikia.nocookie.net`)
   with a visible credit and a link to the source page; only the URLs live in the repo (user decision 2026-11-05).
   Game text (quest titles, descriptions) is read at runtime from the player's own install, never stored.
8. Small PRs: one crate feature or one act of content, each with tests.
