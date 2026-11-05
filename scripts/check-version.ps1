<#
Fails when the workspace version (Cargo.toml [workspace.package]) and src-tauri/tauri.conf.json "version" differ,
or, with -Tag vX.Y.Z, when the tag does not match them. Run by CI and the release workflow.
#>
param([string]$Tag = '')
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$cargo = $null; $inPkg = $false
foreach ($l in Get-Content (Join-Path $repo 'Cargo.toml')) {
    if ($l -match '^\s*\[(.+)\]\s*$') { $inPkg = ($Matches[1] -eq 'workspace.package'); continue }
    if ($inPkg -and $l -match '^\s*version\s*=\s*"([^"]+)"') { $cargo = $Matches[1]; break }
}
$tauri = (Get-Content -Raw (Join-Path $repo 'src-tauri\tauri.conf.json') | ConvertFrom-Json).version
if (-not $cargo) { throw 'no [workspace.package] version in Cargo.toml' }
Write-Host "Cargo.toml workspace version: $cargo; tauri.conf.json version: $tauri"
if ($cargo -ne $tauri) { throw "version mismatch: Cargo.toml has $cargo, tauri.conf.json has $tauri" }
if ($Tag) {
    $t = $Tag -replace '^refs/tags/', '' -replace '^v', ''
    if ($t -ne $cargo) { throw "tag $Tag does not match version $cargo" }
}
Write-Host 'versions in sync'
