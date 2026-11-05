<#
Copies the template saves named in questbook/templates.toml (by stem) plus their .png/.json sidecars from the
read-only fixture folders into <repo>/templates, then verifies SHA-256 of every copied file against its source.
The folder is bundled into the installer (src-tauri/tauri.conf.json bundle.resources). *.sav is gitignored.
Usage: pwsh scripts/collect-templates.ps1 [-Sources <folder>,...]   (default: pack_501, session_V1, session_V2 under $env:W3AL_FIXTURES)
Sources are only read.
#>
param(
    [string[]]$Sources = @()
)
if (-not $Sources) {
    if (-not $env:W3AL_FIXTURES) { throw 'pass -Sources <folder>,... or set W3AL_FIXTURES to your folder of save copies' }
    $Sources = @('pack_501', 'session_V1', 'session_V2') | ForEach-Object { Join-Path $env:W3AL_FIXTURES $_ }
}
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $repo 'templates'
$stems = Select-String -Path (Join-Path $repo 'questbook\templates.toml') -Pattern '^\s*stem\s*=\s*"([^"]+)"' |
    ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -Unique
if (-not $stems) { throw 'no stems found in questbook/templates.toml' }
New-Item -ItemType Directory -Force $dest | Out-Null
$missing = @(); $bytes = 0
foreach ($stem in $stems) {
    $srcSav = $null
    foreach ($s in $Sources) {
        $p = Join-Path $s "$stem.sav"
        if (Test-Path $p) { $srcSav = $p; break }
    }
    if (-not $srcSav) { $missing += $stem; continue }
    $srcDir = Split-Path -Parent $srcSav
    foreach ($ext in 'sav', 'png', 'json') {
        $src = Join-Path $srcDir "$stem.$ext"
        if (-not (Test-Path $src)) { if ($ext -eq 'sav') { throw "missing $src" } else { continue } }
        $dst = Join-Path $dest "$stem.$ext"
        Copy-Item -LiteralPath $src -Destination $dst -Force
        $a = (Get-FileHash -Algorithm SHA256 -LiteralPath $src).Hash
        $b = (Get-FileHash -Algorithm SHA256 -LiteralPath $dst).Hash
        if ($a -ne $b) { throw "hash mismatch for $dst" }
        $bytes += (Get-Item $dst).Length
    }
    Write-Host "ok $stem  (from $srcDir)"
}
if ($missing) { throw ("template saves not found in any source: " + ($missing -join ', ')) }
Write-Host ("{0} templates, {1:N1} MB in {2}" -f @($stems).Count, ($bytes / 1MB), $dest)
