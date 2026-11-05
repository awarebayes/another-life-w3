# Distribution

How Another Life for The Witcher 3 reaches players, what is in each download, and how a
release is cut. Nothing here uploads by itself: the release workflow only makes a **draft**.

## Channels

1. **GitHub Releases (primary, from v0.1.0).** Each release has the NSIS installer, the portable
   zip and `SHA256SUMS`. Release notes carry the unofficial and unsigned warnings.
2. **Nexus Mods (later).** The app is a standalone tool, not a mod: it does not change game
   files, it writes a new save. Post it in the *Utilities* category of The Witcher 3 (and
   mention save editing in the tags). The page needs: a description (use the README "What it
   is" text and the unofficial notice), our own screenshots (story screen, Review screen, the
   result in game), the permissions section (open source, MIT OR Apache-2.0 code and CC BY-SA
   questbook, credits to Atvaark/W3SavegameEditor, no game assets), requirements (Witcher 3
   Remastered PC, patch 5.01, WebView2), and the installer or zip. Check Nexus rules on saves:
   the template saves are derived from other authors' packs, so state their origin on the
   page and keep the repo link. Do this after v0.1 has had real-world testing.

## What is in which artifact

| Artifact | Contents | Size (approx.) |
| --- | --- | --- |
| `Another-Life_<ver>_x64-setup.exe` (NSIS; the workflow renames the space to a dash) | app, `w3al.exe` (CLI), `questbook/`, `templates/` | 12.4 MiB (templates inside) |
| `another-life-w3-<ver>-portable-win64.zip` | `another-life.exe`, `w3al.exe`, `questbook/`, `templates/`, README, licences, NOTICE, player guides | 23.6 MB |
| `SHA256SUMS` | hashes of the two files above | tiny |
| `templates-v1.zip` (separate release `templates-v1`, **not** for players) | template saves and sidecars, flat | 16.1 MB |

The installer puts `w3al.exe` next to the app (`%LOCALAPPDATA%\Another Life\w3al.exe` for a
per-user install). The installer does not add it to PATH; run it by full path or from that
folder. Both the app and the CLI look for `questbook/` and `templates/` next to their own
exe, so no setup is needed in either layout. Overrides: `W3AL_QUESTBOOK`, `--questbook`,
`--template-dirs`.

Examples:

```powershell
cd "$env:LOCALAPPDATA\Another Life"      # or the unzipped portable folder
.\w3al.exe --help
.\w3al.exe lint
.\w3al.exe export --run my.toml --out-dir .
.\w3al.exe edit-plan "C:\path\to\copy.sav"
.\w3al.exe character "C:\path\to\copy.sav"
```

Size budget: keep the installer and the portable zip under ~35 MB each so GitHub and Nexus uploads stay
easy. New templates grow the `templates-vN.zip`; watch it.

## Templates in automated builds

Template saves are third-party and pack-derived, so they are not in git. They live in a
GitHub Release asset and the release workflow downloads them.

One time, and again whenever the template set changes (then use a new name, `templates-v2`,
so old tags still build):

```powershell
$env:W3AL_FIXTURES = "C:\w3-fixtures"
pwsh scripts/collect-templates.ps1                    # fills templates/
pwsh scripts/pack-templates.ps1 -WriteHashFile        # dist/templates-v1.zip + templates.sha256
git add templates.sha256 ; git commit                 # the hash the workflow verifies
gh release create templates-v1 dist/templates-v1.zip --title "Template saves v1" --notes "Build input for release.yml. Not a player download."
```

Keep the `templates-v1` release as a **pre-release** so it does not show as "latest". The
workflow runs `gh release download templates-v1`, compares the SHA-256 to `templates.sha256`
and refuses to build on a mismatch. In a private repo the default `GITHUB_TOKEN` can read it.

## Versioning

One version. `Cargo.toml` `[workspace.package] version` and `src-tauri/tauri.conf.json`
`version` must match, and the tag is `v<version>`. `scripts/check-version.ps1` checks this;
CI runs it on every push and the release workflow runs it with the tag. (`Cargo.lock` follows
from `cargo build`; commit it.)

## Unsigned builds and SmartScreen

The installer and exes are not code signed. On first run Windows SmartScreen shows "Windows
protected your PC": click **More info**, then **Run anyway**. Antivirus tools may also flag
unsigned Tauri/Rust exes occasionally; the repo is open source and `SHA256SUMS` lets users
check the download. Say this in the release notes and the player guide.

Signing later (optional, reputation takes time even when signed):

- Azure Trusted Signing (Artifact Signing): about 10 USD per month, needs an identity-validated
  organisation or individual in supported regions; best fit for CI.
- OV certificate from a CA: roughly 100 to 300 USD per year, SmartScreen reputation builds slowly.
- EV certificate: roughly 250 to 500 USD per year plus a hardware token or cloud HSM; gives
  immediate SmartScreen trust.
- SignPath Foundation offers free signing for qualifying open source projects (apply with the repo).

Tauri supports `bundle.windows.signCommand` / certificate thumbprint for any of these.

## Updates

Manual: players download the new release. Saves already exported are unaffected. A later
"new version available" check can ask the GitHub API (`/repos/awarebayes/another-life-w3/releases/latest`)
and only show a link; keep it opt-in, no background download, no telemetry. The Tauri updater
plugin is not planned (it needs signing keys and a hosted manifest).

## Privacy

The app has no telemetry and no accounts. When online images are enabled it fetches
illustrations from the Witcher Wiki CDN (`static.wikia.nocookie.net`), which sees the player's
IP address and the requested image URLs. It can be turned off in the app and the app works
without it. Saves are read and written only locally; export never overwrites a file.

## Release checklist

1. Land everything on `main`; CI green (`cargo test`, clippy, `w3al lint`).
2. Bump the version in `Cargo.toml` (`[workspace.package]`) **and** `src-tauri/tauri.conf.json`;
   `cargo build` to refresh `Cargo.lock`; `pwsh scripts/check-version.ps1`.
3. Confirm the `templates-v1` release exists and `templates.sha256` matches it (or publish `templates-v2`
   and update `TEMPLATES_TAG` in `.github/workflows/release.yml` plus `templates.sha256`).
4. Commit, tag `vX.Y.Z`, push the tag. The Release workflow runs (about 15 to 25 minutes).
5. Open the **draft** release; download the installer and zip; compare `SHA256SUMS`.
6. Test on a clean Windows user account (no Rust, no repo): install, start (SmartScreen
   click-through), pick a start, export into a copy of a saves folder, load the save in game;
   run `w3al.exe --help` and `lint` from the install folder; uninstall; unzip the portable zip
   elsewhere and repeat the export.
7. Edit the notes (what changed, known limits, unsigned warning), then **Publish**.
8. Later channels (Nexus) only after step 7 has had some real use.


## Verified locally (2026-11-05, private CARGO_TARGET_DIR)

Run on this machine, nothing pushed, tagged or uploaded:

- `scripts/check-version.ps1` (0.1.0 in sync; `-Tag v0.1.0` passes, `-Tag v0.2.0` fails as intended).
- `scripts/pack-templates.ps1`: `templates-v1.zip`, 16.1 MB, 30 files (10 saves with sidecars), flat.
- `scripts/package-portable.ps1`: release build of the CLI and `cargo tauri build --no-bundle`; zip is
  23.6 MB, 162 files (`another-life.exe` 14.5 MB, `w3al.exe` 6.2 MB). Unzipped elsewhere,
  `w3al.exe --help` works and `w3al.exe lint` run from an unrelated folder finds the bundled
  `questbook/` (20 quests, 0 errors).
- `cargo tauri build --config src-tauri/tauri.release.conf.json`: NSIS installer 12.4 MiB; the generated
  `installer.nsi` installs `w3al.exe` (from `src-tauri/bundle-extra/`), `questbook/` and the template saves.
  The installer itself was not run.
- `cargo test` and `cargo clippy --all-targets -- -D warnings` pass.
- Not run locally: the GitHub workflows themselves (no YAML parser or runner here; reviewed by
  hand). The first tag run is the real test, which is why it only creates a draft.

## One-time steps for the maintainer

1. Fill `templates/` (`pwsh scripts/collect-templates.ps1` with `W3AL_FIXTURES` set) and run
   `pwsh scripts/pack-templates.ps1 -WriteHashFile`; commit `templates.sha256` (the workflow fails without it).
2. Upload the zip to a pre-release tagged `templates-v1`:
   `gh release create templates-v1 dist/templates-v1.zip --prerelease --title "Template saves v1" --notes "Build input for release.yml. Not a player download."`
3. Push the tag to get a draft release: `git tag v0.1.0; git push origin v0.1.0` (or run the Release workflow
   by hand with the input `v0.1.0`). Test the draft on a clean account, then publish (checklist above).
