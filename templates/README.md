# templates/

Template saves are **not in git**: they are real, personal Witcher 3 saves. The installer ships
them from this folder at build time.

To fill it, point `W3AL_FIXTURES` at your folder of save copies and run, from the repo root:

```powershell
pwsh scripts/collect-templates.ps1
```

The script copies the saves named in `questbook/templates.toml` (plus their sidecars) here and
verifies a SHA-256 for each. Without them `cargo tauri build` produces an installer that cannot
export. Everything in this folder except this file is gitignored.
