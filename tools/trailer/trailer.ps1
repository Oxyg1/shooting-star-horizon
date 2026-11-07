# The Shooting Star trailer: films the game and cuts the trailer on this computer.
#
#   Right-click this file > Run with PowerShell
#   (or, in PowerShell:  powershell -ExecutionPolicy Bypass -File trailer.ps1)
#
# It installs what is missing (Python and ffmpeg, with winget), then runs
# trailer.py: start recording, then type /tss trailer in the game.
# Extra arguments go to trailer.py, e.g.  -- --offset 0.3
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

function Have($name) { [bool](Get-Command $name -ErrorAction SilentlyContinue) }

function Refresh-Path {
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $links = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Links'
    $env:Path = "$machine;$user;$links"
}

function Install($what, $id) {
    if (-not (Have 'winget')) {
        Write-Host "$what is missing and winget is not available: install $what and run this again." -ForegroundColor Red
        exit 1
    }
    Write-Host "Installing $what..." -ForegroundColor Cyan
    winget install --id $id -e --accept-source-agreements --accept-package-agreements --silent | Out-Host
    Refresh-Path
}

Refresh-Path
$python = $null
foreach ($candidate in @('py', 'python', 'python3')) {
    if (Have $candidate) {
        # The Microsoft Store stub answers but runs nothing.
        $version = & $candidate --version 2>$null
        if ($LASTEXITCODE -eq 0 -and $version -match 'Python 3') { $python = $candidate; break }
    }
}
if (-not $python) {
    Install 'Python' 'Python.Python.3.12'
    foreach ($candidate in @('py', 'python')) { if (Have $candidate) { $python = $candidate; break } }
}
if (-not (Have 'ffmpeg')) {
    Install 'ffmpeg' 'Gyan.FFmpeg'
    if (-not (Have 'ffmpeg')) {
        $found = Get-ChildItem -Path (Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages') -Recurse -Filter ffmpeg.exe -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($found) { $env:Path = "$($found.DirectoryName);$env:Path" }
    }
}
if (-not $python -or -not (Have 'ffmpeg')) {
    Write-Host 'Python or ffmpeg could not be installed. Open a new PowerShell window and run this again.' -ForegroundColor Red
    exit 1
}

Write-Host ''
Write-Host 'The Shooting Star trailer' -ForegroundColor Yellow
Write-Host '1. Start Horizon and open a NEW world made for the trailer (Gravitia sends it to Imaginary Space for good).'
Write-Host '2. Put the game on the main screen, fullscreen.'
Write-Host '3. When this says it is recording, type /tss trailer in the game and leave the mouse alone (about 4 minutes).'
Write-Host ''
& $python (Join-Path $here 'trailer.py') @args
if ($LASTEXITCODE -eq 0) {
    $out = Join-Path $here 'trailer-out'
    Write-Host "Done: $out\shooting-star-trailer.mp4" -ForegroundColor Green
    Start-Process explorer.exe $out
}
Read-Host 'Press Enter to close'
