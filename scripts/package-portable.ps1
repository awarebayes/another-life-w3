<#
Builds the portable zip: dist/another-life-w3-<version>-portable-win64.zip containing
  another-life.exe (the app), w3al.exe (the CLI), questbook/, templates/, README, LICENSE files, NOTICE, docs/PLAYER_GUIDE*.
The CLI and the app both find questbook/ and templates/ next to the executable, so the unzipped folder works anywhere.

Usage: pwsh scripts/package-portable.ps1 [-OutDir <folder>] [-TargetDir <cargo target dir>] [-SkipBuild]
  default: builds the CLI (`cargo build --release -p w3al-cli`) and the app exe (`cargo tauri build --no-bundle`)
  -SkipBuild: use what is already in <TargetDir>/release (the release workflow builds first, with the installer)
  -TargetDir: defaults to $env:CARGO_TARGET_DIR, else <repo>/target
Also stages the CLI for the installer: src-tauri/bundle-extra/w3al.exe (see src-tauri/tauri.release.conf.json).
templates/ must be populated (scripts/collect-templates.ps1 or the templates-v1 release asset), otherwise the zip is refused.
#>
param(
    [string]$OutDir = '',
    [string]$TargetDir = '',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'dist' }
if (-not $TargetDir) { $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' } }
$rel = Join-Path $TargetDir 'release'

& (Join-Path $PSScriptRoot 'check-version.ps1')
$version = (Get-Content -Raw (Join-Path $repo 'src-tauri\tauri.conf.json') | ConvertFrom-Json).version

if (-not (Get-ChildItem (Join-Path $repo 'templates') -Filter *.sav -ErrorAction SilentlyContinue)) {
    throw 'templates/ has no .sav files: run scripts/collect-templates.ps1 or unzip the templates-v1 release asset there'
}
Push-Location $repo
try {
    if (-not $SkipBuild) {
        $env:CARGO_TARGET_DIR = $TargetDir
        cargo build --release --locked -p w3al-cli
        if ($LASTEXITCODE) { throw 'cargo build (cli) failed' }
        cargo tauri build --no-bundle
        if ($LASTEXITCODE) { throw 'cargo tauri build failed' }
    }
} finally { Pop-Location }

$app = Join-Path $rel 'w3al-app.exe'
$cli = Join-Path $rel 'w3al.exe'
foreach ($f in $app, $cli) { if (-not (Test-Path $f)) { throw "missing $f (build first or drop -SkipBuild)" } }

# Stage the CLI for the NSIS installer resource.
New-Item -ItemType Directory -Force (Join-Path $repo 'src-tauri\bundle-extra') | Out-Null
Copy-Item $cli (Join-Path $repo 'src-tauri\bundle-extra\w3al.exe') -Force

$name = "another-life-w3-$version-portable-win64"
$stage = Join-Path $OutDir "stage\$name"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $app (Join-Path $stage 'another-life.exe')
Copy-Item $cli (Join-Path $stage 'w3al.exe')
Copy-Item -Recurse (Join-Path $repo 'questbook') (Join-Path $stage 'questbook')
New-Item -ItemType Directory (Join-Path $stage 'templates') | Out-Null
Get-ChildItem (Join-Path $repo 'templates') -File | Where-Object { $_.Name -ne 'README.md' } | Copy-Item -Destination (Join-Path $stage 'templates')
foreach ($f in 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE.md') { Copy-Item (Join-Path $repo $f) $stage }
New-Item -ItemType Directory (Join-Path $stage 'docs') | Out-Null
foreach ($f in 'PLAYER_GUIDE.md', 'PLAYER_GUIDE_RU.md') {
    $p = Join-Path $repo "docs\$f"; if (Test-Path $p) { Copy-Item $p (Join-Path $stage 'docs') }
}
@"
Another Life for The Witcher 3 - portable build $version (unofficial fan tool)

another-life.exe   the app (needs the WebView2 runtime; Windows 11 has it)
w3al.exe           the command line tool: open a terminal in this folder and run  .\w3al.exe --help
questbook/         story content (CC BY-SA 4.0)
templates/         template saves the export patches (never overwritten, never written to)

The exe is not code signed, so Windows SmartScreen may warn on first start: More info, Run anyway.
Back up your saves first; close the game before exporting. See README.md and docs/PLAYER_GUIDE.md.
"@ | Set-Content -Encoding utf8 (Join-Path $stage 'START_HERE.txt')

New-Item -ItemType Directory -Force $OutDir | Out-Null
$zip = Join-Path $OutDir "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
# Entries are written by hand with forward slashes: ZipFile.CreateFromDirectory on Windows PowerShell 5.1 writes backslashes.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$za = [IO.Compression.ZipFile]::Open($zip, 'Create')
try {
    $base = (Split-Path $stage -Parent).TrimEnd('\') + '\'
    foreach ($f in Get-ChildItem -LiteralPath $stage -Recurse -File) {
        $entry = $f.FullName.Substring($base.Length).Replace('\', '/')
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($za, $f.FullName, $entry, [IO.Compression.CompressionLevel]::Optimal)
    }
} finally { $za.Dispose() }
Remove-Item -Recurse -Force (Join-Path $OutDir 'stage')
$h = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
Write-Host ("{0}  {1}  ({2:N1} MB)" -f $h, (Split-Path $zip -Leaf), ((Get-Item $zip).Length / 1MB))
