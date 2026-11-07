# Builds the release exe and packs it as dist\bt-w5-control-<version>-windows-x64.zip, the
# asset the Scoop bucket (roosmsg/scoop-bucket) installs from.
#   pwsh -File scripts\package.ps1
$ErrorActionPreference = 'Stop'

if (-not $env:CARGO_HOME) { $env:CARGO_HOME = "$env:USERPROFILE\scoop\persist\rustup-gnu\.cargo" }
if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = "$env:USERPROFILE\scoop\persist\rustup-gnu\.rustup" }
$env:PATH = "$env:CARGO_HOME\bin;$env:USERPROFILE\scoop\apps\mingw\current\bin;$env:PATH"

$repo = Split-Path $PSScriptRoot -Parent
$version = (Select-String -Path (Join-Path $repo 'Cargo.toml') -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value

Push-Location $repo
try { cargo build --release; if ($LASTEXITCODE) { throw 'cargo build failed' } } finally { Pop-Location }

$dist = Join-Path $repo 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$zip = Join-Path $dist "bt-w5-control-$version-windows-x64.zip"
Remove-Item $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $repo 'target\release\bt-w5-control.exe') -DestinationPath $zip
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$zip"
"sha256 $hash"
