# Another Life for The Witcher 3

Walk The Witcher 3's story as an illustrated text RPG, make the real choices, then wake up in
Vizima with a save whose world state matches. Think Skyrim's *Alternate Start - Live Another
Life*, except you decide the past: who lived, who joined you, who you loved.

<!-- TODO: add a screenshot of the story screen and the Review screen here (docs/img/, own screenshots only). -->

*Unofficial fan tool. Not affiliated with or endorsed by CD PROJEKT RED. The Witcher is a
trademark of CD PROJEKT S.A. No game files are included. See [NOTICE.md](NOTICE.md).*

## Status

Early software, honestly:

- **Proven in game** (on the author's own patch 5.01 Remastered PC saves): an unedited repack
  loads and plays; a changed game fact is read by the game and survives the game's own re-save
  (for example the Gwent difficulty setting visibly changes); a quest graph transplant makes
  a quest count as done in play (several quests, two branches, a never-started quest).
- **Not proven**: position and world edits, DLC and mod state, other game patches, and most of
  the questbook. Quests in the sample content still carry `PLACEHOLDER_` effects and the
  exporter refuses those on purpose.
- Content: acts and quests are being researched and written act by act. Everything is traced
  to a script, a quest graph or a diff between two real saves; nothing is guessed.

Phase details live in [CLAUDE.md](CLAUDE.md), findings in [docs/research.md](docs/research.md).

## Features

- Illustrated, second-person retelling of the main story and side quests (original text).
- Each answer states plainly what it changes in the world; defaults if you do not care.
- Review screen before export.
- Export writes one **new** save (`ManualSave_*.sav`) next to your saves; it never overwrites.
- CLI (`w3al`) for inspecting, diffing and validating saves and the questbook.
- English and Russian UI.

Out of scope for v1: level and gold (the in-game console already does it on any save).

## Install

- **Player:** download the Windows installer from the repository's Releases page, run it, and
  read the [player guide](docs/PLAYER_GUIDE.md) ([Russian](docs/PLAYER_GUIDE_RU.md)).
  Needs The Witcher 3 Remastered (PC, patch 5.01) and the WebView2 runtime (ships with Windows 11).
- **From source:** needs Rust stable (MSVC toolchain on Windows) and the Tauri CLI.

```bash
cargo test                                         # w3save, w3al-core, w3al-cli, w3graph
cargo run -p w3al-cli -- lint                      # validate the questbook
cargo install tauri-cli --version "^2" --locked
cargo tauri dev                                    # run the app
cargo tauri build                                  # Windows installer (NSIS)
```

**Template saves are not in git.** The exporter patches a real template save, and those are
personal files, so the installer picks them up from `templates/` at build time. To build a
working installer, put copies of your own saves in a folder, set `W3AL_FIXTURES` to it and run
`pwsh scripts/collect-templates.ps1` (see [templates/README.md](templates/README.md)).
A build without them runs, but cannot export.

CLI: `cargo run -p w3al-cli -- <dump|roundtrip|diff|facts|lint|check-save|export|edit-fact|journal|journal-diff|journal-set|tree|graph-diff|questbook-json> ...`

## How it works

1. The app walks the questbook (TOML) and records your answers.
2. Answers resolve to effects: game facts, journal entries and quest graph state.
3. The export copies a template save that is close to your chosen starting point, patches the
   effects into it (`w3save` reads and writes the container, variable table and token tree,
   and rebases all offsets), and writes the result as a new `.sav` plus sidecars.

## Layout

`crates/w3save` (save format) - `crates/w3graph` (quest graphs of your own install) -
`crates/w3al-core` (questbook, runs, effects, export) - `crates/w3al-cli` -
`src-tauri` + `ui` (app) - `questbook` (TOML content, CC BY-SA).
Start with [CLAUDE.md](CLAUDE.md), [DECISIONS.md](DECISIONS.md) and [CONTRIBUTING.md](CONTRIBUTING.md).

## Safety

- Existing saves are never overwritten (files are created with `create_new`).
- Back up `Documents\The Witcher 3\gamesaves` before trying it, and close the game first.
- Never run experiments on files in the game's own saves folder; use copies.
- Real saves are never committed to this repository (`*.sav` is gitignored).

## Licence

- Code: [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE), at your option.
- Questbook content (`questbook/`): [CC BY-SA 4.0](questbook/LICENSE).
- Third-party credits: [NOTICE.md](NOTICE.md).

## Disclaimer

This is an unofficial fan project, not affiliated with or endorsed by CD PROJEKT RED.
The Witcher is a trademark of CD PROJEKT S.A. Illustrations are linked from the Witcher Wiki
and remain the property of their owners. Use at your own risk and keep backups of your saves.
