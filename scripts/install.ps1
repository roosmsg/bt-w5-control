# Builds the release binary, copies it to %LOCALAPPDATA%\Programs\BT-W5 Control and adds a
# Start menu shortcut. Starting the app once registers the logon entry itself.
# Also removes an install under the earlier name "W5 Control".
#   pwsh -File scripts\install.ps1 [-Launch]
param([switch]$Launch)
$ErrorActionPreference = 'Stop'

# Scoop's rustup-gnu sets these as user variables; shells started before that need them here.
if (-not $env:CARGO_HOME) { $env:CARGO_HOME = "$env:USERPROFILE\scoop\persist\rustup-gnu\.cargo" }
if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = "$env:USERPROFILE\scoop\persist\rustup-gnu\.rustup" }
$env:PATH = "$env:CARGO_HOME\bin;$env:USERPROFILE\scoop\apps\mingw\current\bin;$env:PATH"

$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
try { cargo build --release; if ($LASTEXITCODE) { throw 'cargo build failed' } } finally { Pop-Location }

$programs = Join-Path $env:LOCALAPPDATA 'Programs'
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$target = Join-Path $programs 'BT-W5 Control'
$exe = Join-Path $target 'bt-w5-control.exe'

Get-Process bt-w5-control, w5-control -ErrorAction SilentlyContinue | Stop-Process -Force

# Earlier name: folder, shortcut and logon entry.
Remove-Item (Join-Path $programs 'W5 Control') -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $startMenu 'W5 Control.lnk') -Force -ErrorAction SilentlyContinue
Remove-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'W5 Control' -ErrorAction SilentlyContinue

New-Item -ItemType Directory -Force $target | Out-Null
Copy-Item (Join-Path $repo 'target\release\bt-w5-control.exe') $exe -Force

$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut((Join-Path $startMenu 'BT-W5 Control.lnk'))
$link.TargetPath = $exe
$link.WorkingDirectory = $target
$link.Description = 'BT-W5 Control'
$link.Save()

"Installed to $target"
if ($Launch) { Start-Process $exe }
