<#
.SYNOPSIS
  The Shooting Star (Demo) for Horizon: installer for Windows.

.DESCRIPTION
  One line in PowerShell:

    irm https://raw.githubusercontent.com/Oxyg1/shooting-star-horizon/main/install/install.ps1 | iex

  1. Downloads the original mod (rimuru.dev, All Rights Reserved) from CurseForge, as you would:
     this repository carries none of its files.
  2. Builds the content mod from it (items, blocks, sounds, the fx shaders, textures, the films'
     camera tables) into your Horizon game folder and turns it on.
  3. Installs the native mod (the port's Rust code) from GitHub with the Horizon Launcher and builds
     the game with it.

.PARAMETER GameDir
  The Horizon game folder (default: $env:HORIZON_HOME, else %APPDATA%\horizon).
.PARAMETER Launcher
  horizon-launcher.exe (default: the usual install folder, or the PATH).
.PARAMETER Local
  Build the native mod from this checkout instead of GitHub.
.PARAMETER ContentOnly
  Install the content mod only.
.PARAMETER NoBuild
  Install, but build the game later.
#>
param(
  [string]$GameDir = "",
  [string]$Launcher = "",
  [switch]$Local,
  [switch]$ContentOnly,
  [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$Repo = if ($env:SHOOTING_STAR_REPO) { $env:SHOOTING_STAR_REPO } else { 'Oxyg1/shooting-star-horizon' }
$Ref = if ($env:SHOOTING_STAR_REF) { $env:SHOOTING_STAR_REF } else { 'main' }
$ModId = 'shooting_star_demo'
# The original's files on CurseForge (project 1719195), checked by SHA-256.
$Jars = @{
  '1.5.2' = @{ Url = 'https://www.curseforge.com/api/v1/mods/1719195/files/9438887/download'
               Sha = '90D3C145EA00A3A5C4119B51F42B06CD0569C83733D2F3A5398579E77F4D703A'
               File = 'the-shooting-star-demo-1.5.2-fabric.jar'
               Inner = 'META-INF/jars/the-shooting-star-demo-1.5.2-demo+mc26.3.jar' }
  '1.4.4' = @{ Url = 'https://www.curseforge.com/api/v1/mods/1719195/files/9231726/download'
               Sha = '1C361965467E66C0050609F5F9CE53EC4A70677EC74DC5F119DA79BDC46EFA1A'
               File = 'the-shooting-star-demo-1.4.4-fabric.jar'
               Inner = 'META-INF/jars/the-shooting-star-demo-1.4.4-demo+mc26.3.jar' }
}

function Say($text) { Write-Host "* $text" -ForegroundColor Magenta }

# A program run with its output kept. Windows PowerShell 5.1 turns whatever a
# program writes to stderr (`java -version` writes there) into errors that
# 'Stop' throws on, so programs are started directly, not with `&`.
function Invoke-Program([string]$Exe, [string[]]$Arguments) {
  $info = New-Object System.Diagnostics.ProcessStartInfo
  $info.FileName = $Exe
  $quoted = foreach ($a in $Arguments) { if ($a -match '[\s"]') { '"' + ($a -replace '"', '\"') + '"' } else { $a } }
  $info.Arguments = $quoted -join ' '
  $info.UseShellExecute = $false
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $info.CreateNoWindow = $true
  $process = [System.Diagnostics.Process]::Start($info)
  $stdout = $process.StandardOutput.ReadToEndAsync()
  $stderr = $process.StandardError.ReadToEnd()
  $process.WaitForExit()
  [pscustomobject]@{ Code = $process.ExitCode; Out = $stdout.Result; Err = $stderr }
}

# A program run in front of the player (the launcher's own progress), whose stderr is not an error.
function Invoke-Shown([string]$Exe, [string[]]$Arguments) {
  $ErrorActionPreference = 'Continue'
  & $Exe @Arguments | Out-Host
  return $LASTEXITCODE
}
function Die($text) { Write-Host "x $text" -ForegroundColor Red; throw $text }

if (-not $GameDir) {
  $GameDir = if ($env:HORIZON_HOME) { $env:HORIZON_HOME } else { Join-Path $env:APPDATA 'horizon' }
}
$Work = Join-Path ([IO.Path]::GetTempPath()) ("shooting-star-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force -Path $Work | Out-Null

try {
  # This checkout, or the repository downloaded.
  $Src = $null
  if ($PSScriptRoot -and (Test-Path (Join-Path $PSScriptRoot '..\native\data\files.tsv'))) {
    $Src = Split-Path -Parent $PSScriptRoot
  } else {
    Say "Downloading $Repo ($Ref)"
    $zip = Join-Path $Work 'repo.zip'
    Invoke-WebRequest -UseBasicParsing -Uri "https://codeload.github.com/$Repo/zip/$Ref" -OutFile $zip
    Expand-Archive -Path $zip -DestinationPath (Join-Path $Work 'repo')
    $Src = (Get-ChildItem (Join-Path $Work 'repo') -Directory | Select-Object -First 1).FullName
  }

  # The original's jars, kept between runs.
  $Cache = Join-Path $env:LOCALAPPDATA 'shooting-star-horizon'
  New-Item -ItemType Directory -Force -Path $Cache | Out-Null
  $Inner = @{}
  foreach ($version in $Jars.Keys) {
    $jar = $Jars[$version]
    $path = Join-Path $Cache $jar.File
    if (-not ((Test-Path $path) -and ((Get-FileHash $path -Algorithm SHA256).Hash -eq $jar.Sha))) {
      Say "Downloading $($jar.File) from CurseForge"
      Invoke-WebRequest -UseBasicParsing -Uri $jar.Url -OutFile "$path.part"
      if ((Get-FileHash "$path.part" -Algorithm SHA256).Hash -ne $jar.Sha) { Die "$($jar.File): not the file this port was made for (checksum)" }
      Move-Item -Force "$path.part" $path
    }
    $outer = [IO.Compression.ZipFile]::OpenRead($path)
    try {
      $entry = $outer.GetEntry($jar.Inner)
      if (-not $entry) { Die "$($jar.File) has no $($jar.Inner)" }
      $innerPath = Join-Path $Work ($version.Replace('.', '') + '.jar')
      [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $innerPath, $true)
      $Inner[$version] = $innerPath
    } finally { $outer.Dispose() }
  }

  # The content mod: the port's own files, then the original's.
  Say 'Building the content mod'
  $Out = Join-Path $Work 'content'
  Copy-Item (Join-Path $Src 'native\data\content') $Out -Recurse
  $opened = @{}
  foreach ($version in $Inner.Keys) { $opened[$version] = [IO.Compression.ZipFile]::OpenRead($Inner[$version]) }
  $count = 0
  try {
    foreach ($line in Get-Content (Join-Path $Src 'native\data\files.tsv')) {
      if (-not $line -or $line.StartsWith('#')) { continue }
      $dest, $version, $from = $line.Split("`t")
      $entry = $opened[$version].GetEntry($from)
      if (-not $entry) { Die "$from is missing from the $version jar" }
      $target = Join-Path $Out ($dest.Replace('/', '\'))
      New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
      [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $true)
      $count++
    }
  } finally { foreach ($z in $opened.Values) { $z.Dispose() } }
  Say "  $count of the original's files"

  # The launcher.
  if (-not $Launcher) {
    $usual = Join-Path $env:LOCALAPPDATA 'Programs\Horizon Launcher\horizon-launcher.exe'
    $onPath = Get-Command horizon-launcher -ErrorAction SilentlyContinue
    if (Test-Path $usual) { $Launcher = $usual }
    elseif ($onPath) { $Launcher = $onPath.Source }
  }

  # The films' camera tables: the original's path classes, run once (Java 25, as Minecraft 26.3's).
  $Java = $null
  $candidates = @()
  if ($env:JAVA_HOME) { $candidates += Join-Path $env:JAVA_HOME 'bin\java.exe' }
  $onPath = Get-Command java -ErrorAction SilentlyContinue
  if ($onPath) { $candidates += $onPath.Source }
  foreach ($candidate in $candidates) {
    if ($candidate -and (Test-Path $candidate)) {
      $version = Invoke-Program $candidate @('-version')
      if (($version.Err + $version.Out) -match 'version "(\d+)' -and [int]$Matches[1] -ge 25) { $Java = $candidate; break }
    }
  }
  if (-not $Java -and $Launcher) {
    try {
      $runtime = (Invoke-Program $Launcher @('--json', 'java', 'install', '25')).Out | ConvertFrom-Json
      $candidate = Join-Path $runtime.home 'bin\java.exe'
      if (Test-Path $candidate) { $Java = $candidate }
    } catch { }
  }
  if ($Java) {
    Say "Recording the films' cameras (Java: $Java)"
    $films = Join-Path $Out "data\$ModId\films"
    $tool = Join-Path $Src 'native\data\films-tool'
    $run = Invoke-Program $Java @('-cp', ("{0};{1}" -f $tool, $Inner['1.5.2']), 'Films', $films)
    if ($run.Code -ne 0) { Write-Host $run.Err; Die "the films' tables could not be made" }
    $run = Invoke-Program $Java @('-cp', ("{0};{1}" -f $tool, $Inner['1.4.4']), 'Films', $films, '--tesseract')
    if ($run.Code -ne 0) { Write-Host $run.Err; Die "the Tesseract's sky table could not be made" }
  } else {
    Say 'No Java 25 found: the films (the cutscenes in space) stay off. Install Horizon Launcher, or Java 25, and run this again.'
  }

  # Into the game folder, turned on.
  $mods = Join-Path $GameDir 'horizon-mods'
  $target = Join-Path $mods $ModId
  New-Item -ItemType Directory -Force -Path $mods | Out-Null
  if (Test-Path $target) { Remove-Item $target -Recurse -Force }
  Copy-Item $Out $target -Recurse
  $list = Join-Path $GameDir 'horizon-mods.txt'
  $lines = [Collections.Generic.List[string]]::new()
  if (Test-Path $list) { foreach ($l in Get-Content $list) { if ($l -notmatch "^mod\.$($ModId):") { $lines.Add($l) } } }
  $lines.Add("mod.$($ModId):true")
  [IO.File]::WriteAllLines($list, $lines)
  Say "Content mod installed: $target"

  if ($ContentOnly) { Say 'Done (content only).'; return }
  if (-not $Launcher) { Die 'Horizon Launcher was not found: pass -Launcher "C:\path\to\horizon-launcher.exe"' }

  # The native mod: the port's code, built into the game by the launcher.
  if ($Local) {
    Say "Building the native mod from $Src\native"
    if (-not $NoBuild -and (Invoke-Shown $Launcher @('native-build', (Join-Path $Src 'native'), '--trust-native-code')) -ne 0) { Die 'the build failed: see the launcher''s native/build.log' }
  } else {
    Say "Installing the native mod from github.com/$Repo"
    if ((Invoke-Shown $Launcher @('mods', 'add-github', "https://github.com/$Repo", '--trust-native-code')) -ne 0) { Die 'the launcher could not install the native mod' }
    if (-not $NoBuild -and (Invoke-Shown $Launcher @('native-build', '--trust-native-code')) -ne 0) { Die 'the build failed: see the launcher''s native/build.log' }
  }
  Say 'Done. Open Horizon Launcher and press Play: the Stellar Remote is in the creative menu.'
} finally {
  Remove-Item $Work -Recurse -Force -ErrorAction SilentlyContinue
}
