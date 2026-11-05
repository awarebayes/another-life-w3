<#
Zips templates/ (the saves and sidecars that scripts/collect-templates.ps1 copied there) into
dist/templates-v1.zip and prints the SHA-256, ready to upload as a GitHub Release asset on the tag `templates-v1`.
With -WriteHashFile the hash is also written to templates.sha256 at the repo root (tracked; one line
"<hash>  templates-v1.zip"); the release workflow checks the downloaded asset against it.
Usage: pwsh scripts/pack-templates.ps1 [-Name templates-v1] [-OutDir <folder>] [-WriteHashFile]
Nothing is uploaded. Upload by hand:  gh release create templates-v1 dist/templates-v1.zip --title "Template saves v1" --notes "..."
Bump the name (templates-v2) whenever the template set changes, so old release tags keep building.
#>
param(
    [string]$Name = 'templates-v1',
    [string]$OutDir = '',
    [switch]$WriteHashFile
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $repo 'dist' }
$src = Join-Path $repo 'templates'
$files = Get-ChildItem -LiteralPath $src -File | Where-Object { $_.Name -ne 'README.md' }
if (-not ($files | Where-Object { $_.Extension -eq '.sav' })) { throw "no .sav files in $src (run scripts/collect-templates.ps1 first)" }
New-Item -ItemType Directory -Force $OutDir | Out-Null
$zip = Join-Path $OutDir "$Name.zip"
if (Test-Path $zip) { Remove-Item -LiteralPath $zip -Force }
# Flat zip (files at the root) so it unpacks straight into templates/.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$za = [IO.Compression.ZipFile]::Open($zip, 'Create')
try { foreach ($f in $files) { [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($za, $f.FullName, $f.Name, [IO.Compression.CompressionLevel]::Optimal) } } finally { $za.Dispose() }
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLower()
$size = (Get-Item $zip).Length
Write-Host ("{0}  {1}  ({2:N1} MB, {3} files)" -f $hash, "$Name.zip", ($size / 1MB), $files.Count)
if ($WriteHashFile) {
    $line = "$hash  $Name.zip"
    Set-Content -LiteralPath (Join-Path $repo 'templates.sha256') -Value $line -Encoding ascii
    Write-Host "wrote templates.sha256 (commit it, then upload $zip to the release tagged $Name)"
}
