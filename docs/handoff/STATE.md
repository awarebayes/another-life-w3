# Project state and handoff (2026-11-04, end of day)

For any agent (Claude, Codex) picking this up. Plan: `W3_Another_Life___Plan.md` and brief `W3AL_BRIEF_for_Sonnet.md` (originals outside the repo, in a private scratch folder). Rules: `CLAUDE.md` (wins for process), `DECISIONS.md` (assumptions), `docs/research.md` (every finding, dated, newest last), `docs/findings/*.md` (deep dives).

## What this is
Open-source Rust tool: the player walks Witcher 3's story as a text RPG, then we write one `.sav` (Remastered PC, patch 5.01) whose world matches their choices. Template-and-patch, never synthesize a save. Tauri 2 shell + plain HTML/CSS/JS `ui/`. Questbook = hand-curated TOML in `questbook/` (CC BY-SA); code MIT/Apache-2.0. Out of v1: Hearts of Stone, Blood and Wine, console saves, level/gold/gear/skills editing, Gwent.

## Layout and commands
Workspace crates: `crates/w3save` (all game-format knowledge), `crates/w3al-core` (questbook, validation, run, export; never touches bytes), `crates/w3al-cli` (binary `w3al`), `src-tauri` (excluded from default members), `ui/`, `questbook/`.

```powershell
cargo build ; cargo test ; cargo clippy --all-targets      # must stay clean (0 warnings)
cargo run -q -p w3al-cli -- lint                           # questbook: 0 errors
# CLI (target\debug\w3al.exe): dump | roundtrip | check-save | facts | journal | journal-diff |
#   journal-set | edit-fact | export | diff | lint | questbook-json
$env:W3AL_FIXTURES = "C:\w3-extract\lukwraith-26oct"       # fixture-gated tests skip when unset
```
Shell notes: Windows, no Python. PowerShell 5.1 (no `&&`) and Git Bash both exist. `tar.exe` handles zip/rar. `System.Drawing` works in PowerShell for thumbnail stamping.

## Status
- **Phase 0 (save-format gate): PASSED for fact edits** (2026-11-04). Proven in game by the user: unedited repack loads; fact appended/changed/removed (`gwent_difficulty` 3 = Hard, removed = Normal); the game's own re-saves keep our facts; same-world teleport, world swap to another visited world (Kaer Morhen, White Orchard).
- **Phase 1 (`w3save`)**: container, payload map, typed token tree (byte-exact on 101 files), facts (parse/set/remove/exact restore), `rebase`, `apply` for facts, journal editor (offline byte-exact reproduction of a real quest completion; in game: shows completed on load), position edit (tested). Not done: quest graph (`questSystem`), `JS_Failed`/new name-table names in game, DLC/mod checks in export.
- **Phase 2 (Tauri shell)**: scaffold works; export must be wired to the real edits; default output name is now `ManualSave_<a>_<b>_<c>` (generated).
- **Phase 3+ (content)**: not started on purpose; do not start until the quest-graph question is answered. Sample questbook uses `PLACEHOLDER_` effect names.

## Key facts about the save format (see docs for evidence)
- Container `SNFHFZLC`: 12-byte chunk records, raw LZ4 chunks, data start 3,084. The last record's end offset is stale/don't-care (game reuses a header buffer); write 0.
- Payload `SAV3`: variable table (unordered, nested), `RB` (name index + offset of each top-level `BS`), `NM`/`MANU` name table (count, zero, 1-byte-length Latin-1 names, zero, `ENOD`), footer `SE`. No checksum found.
- Facts: one `SS` blob (`SXAP`, `SBDF`, count, facts, `EBDF`); fact = packed string, u16 c1 (= number of timed records), u16 n, n x (u16 value, f32 time, f32 expiry). `FactsSet` = remove then add one permanent record.
- Quest log: `CJournalManager > JActiveEntries` (append-only `SJournalEntryStatus`, GUID paths, `EJournalStatus`), `CWitcherJournalManager` counters/tracked quest. Position: `universe > Player > position/rotation`; world: `worldInfo > world`.
- **Open core problem:** the quest graph `questSystem` decides what happens next; journal + facts only decide what is shown (gate J/J2: quest showed completed, but talking to Napp reactivated it). See `docs/research.md` (2026-11-04 night) and `docs/handoff/PROMPT_quest_graph_opus.md`.
- **Gate G (2026-11-05): PASSED for mq0005.** Transplanting the quest's graph scopes + spawn set/stub (with journal + facts) from a later save of the same playthrough makes the quest count as done in play (Napp gone, no reopen).
- File-name ids: `ManualSave_<a>_<b>_<c>`: `b`/`c` = packed local date/time (high/low word of the last `saveInfo` time); `a` follows the tracked quest (cannot compute; keep the template's). The Load list shows quest names only for this name shape.

## Where things are on disk (outside the repo)
- `C:\w3-extract\` third-party 5.01 saves (lukwraith 45 consecutive, imz, hosamnasr (modded, NG+), WestAard). Treat as untrusted read-only. `C:\w3-fixtures\` copies of the user's own saves (+ `after_gate*` folders = the game's re-saves from our tests). `C:\w3-out\` our generated test sets (`gate`, `gate_D`, `gate_T`, `gate_J`).
- Backups: `C:\w3-backup\gamesaves_full_20261104` (full verified copy of the user's saves folder), `C:\w3-backup\user_saves_stash` (the user's 37 own save files, moved out of `gamesaves`), `C:\w3-backup\test_saves_removed_20261104` (earlier test saves), `C:\w3-backup\restore_user_saves.ps1` (puts the user's saves back, never overwrites).
- Scratch tools from research agents: `<scratch>/` (throwaway; the repo has the real tools).

## Test-session rules (the user asked for these)
1. The game's saves folder `<Documents>\The Witcher 3\gamesaves` must hold ONLY the saves needed for the current test (the user does not want to dig through the Load list). It was emptied (except Steam's `steam_autocloud.vdf`, leave it). Steam Cloud may sync deletions: warn the user.
2. Never hard-delete; MOVE files to `C:\w3-backup\...` with a verified backup first. Confirm `witcher3.exe` is not running before moving or copying.
3. Copy test sets in only when the user says so. New names, never overwrite. Label thumbnails (228x128 24bpp PNG, big coloured band with white bold text) so the sets stand out.
4. Give the user (Russian-speaking; messages from now on are in Russian) very short numbered steps with explicit PASS/FAIL criteria. After the test, read the game's re-saves with `w3al facts` / `journal-diff` from a copy.
5. Orchestrator style: delegate big tasks to subagents (Opus is allowed for unclear/hard problems), then verify reports yourself (`cargo test`, `clippy`, a spot-check of a claim) before saying something is done.

## Cleanup status
DONE (user closed the game): the gate J sets and the saves the user made while testing (33 files, 11 stems) were moved from `gamesaves` to `C:\w3-backup\test_saves_removed_20261104_J` (hashes verified). `gamesaves` now holds only `steam_autocloud.vdf`. Copies of the user's J/J2 play saves are also in `C:\w3-fixtures\after_gate_J` and `after_gate_J2`. The user's own saves are in `C:\w3-backup\user_saves_stash`.

## Next tasks (in order)
1. Quest graph research + prototype (`docs/handoff/PROMPT_quest_graph_opus.md`). DONE offline 2026-11-05 (`docs/findings/quest-graph-design.md`,
   `w3save::graph`, `w3al tree | graph-diff`, `journal-set --graph-*`); in-game gate G PASSED for mq0005 on 2026-11-05 (G0, G1; G2 not needed); then P (Precious Cargo), S (spared-Napp branch, donor
   played by the user, `C:\w3-fixtures\donor_S`) and F (granny, never-started quest) passed; R (two donors on one template, `C:\w3-out\gate_R`)
   passed too. Name remap (`NameMode::Remap`, `journal-set --graph-remap`): gate M passed in game (remapped CName); other value types offline only. Quest catalogue of lukwraith: see research.md.
   The user's re-saves go to `C:\w3-fixtures\after_gate_G`; read them with `graph-diff`, `journal-diff`, `facts --diff`.
2. Wire `w3al-core` export so questbook journal/fact/graph effects reach the save. DONE 2026-11-05 (`docs/findings/export-wiring.md`): G1 and R1 reproduced byte for byte from test questbooks; placeholders still refused.
3. Test `JS_Failed` / adding names to the NM table in game; DLC/mod checks for templates (what a vanilla template needs is in `docs/findings/format-unknowns.md`).
4. Only after 1-3: Phase 3 content (plan: `docs/plan/act1-white-orchard.md`) (fact names come from `content\content0\scripts` and save diffs, never from memory).

## Status at 2026-11-05 (night)
- Proven in game today: gates G, P, S, F, R, M (quest completion and branches by journal + facts + quest graph + community transplant; two
  donors on one template; name remap). Export from questbook reproduces G1/R1 byte for byte.
- Act 1: W2 questions out of v1, no choices (`docs/plan/act1-white-orchard.md` section 7).
- Act 2: donors for spirit / Keira / Letho captured by the user from the Nexus 2394 pack (`C:\w3-fixtures\session_V1|V2`, `BRANCHES.txt`);
  content model "decision windows" + stub identity by `idTag` (`docs/plan/decision-chaining.md`). Gates X1 (Keira to KM + Letho invited on one
  template; the game then finished Keira's quest itself) and K2 (Keira to Radovid synthesized from the quest graph, never played) PASSED in game
  2026-11-05 (day); re-saves in `C:\w3-fixtures\after_gate_XK`, details in `docs/research.md`.
- Phase 2: installer builds (`target/release/bundle/nsis/Another Life_0.1.0_x64-setup.exe`); no-effect export payload-identical; UI click-through by a tester pending.
- 2026-11-05 evening: all questbook decisions are synthesized from the game files (no donors; `docs/plan/no-human-pipeline.md`); gate Y1-Y3 PASSED in game; Y4-Y5 accepted without play (user).
- Kaer Morhen allies done (template synthesized from pack 43 through Ugly Baby, `questbook/05_kaer_morhen/ugly_baby.toml`; 4 independent invitations; exports VERIFIED, not loaded in game).
- Gate Z (story chain + start kit) PASSED in game 2026-11-05.
- Next: pack 12 re-save for the Keira date; app click-through; runner mod.
