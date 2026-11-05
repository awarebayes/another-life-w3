# Contributing

Thanks for helping. The project is a Rust workspace plus a plain HTML/JS UI and a TOML
questbook. Start with [CLAUDE.md](CLAUDE.md) (process rules) and [DECISIONS.md](DECISIONS.md).

## Ground rules

1. **Never commit saves, sidecars or game files.** No `.sav`, no screenshots or JSON next to a
   save, no game bundles, scripts, strings or dumps, no text or images copied from the game or
   the Witcher Wiki. `.gitignore` covers the common cases; check `git status` anyway.
   Narration is original, second person. Illustrations are linked, never stored.
2. **Never guess fact names, offsets or quest ids.** Trace each one to a script line, a quest
   graph, or a diff between two real saves. If you cannot, add an open question to
   `docs/research.md` and mark the TOML effect `placeholder = true` with a `PLACEHOLDER_` name.
3. **Never overwrite an existing save.** Export writes a new file with `create_new`.
4. **Do not skip an exit test.** A phase is done when its test in the plan passes, in game
   where the plan says so. Mark unconfirmed format details with `HYPOTHESIS:`.
5. Reference projects are for understanding. Port only MIT/Apache-licensed code, with
   attribution (see NOTICE.md). Do not copy code from unlicensed repositories.
6. Small pull requests: one crate feature or one act of content, with tests.

## Build and test

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -p w3al-cli -- lint          # validates questbook/, must report 0 errors
```

Many tests read real saves. They skip (and say so) when the environment is not set up, so
`cargo test` passes on a clean checkout. To run them, put **copies** of your own saves in a
folder outside the repo and point the tests at it:

```powershell
$env:W3AL_FIXTURES = "D:\w3-fixtures"          # copies, never the game's gamesaves folder
$env:W3AL_INSTALL  = "D:\Games\The Witcher 3"  # optional: your game install, read only
cargo test
```

Optional extras some tests read: `W3AL_DONORS`, `W3AL_GATES`, `W3AL_EXTRA_FIXTURES`.

## Content

Quest TOML lives in `questbook/` (schema: `questbook/SCHEMA.md`, licence CC BY-SA 4.0 by
contributing you agree to it). Run `w3al lint` before opening a PR.
