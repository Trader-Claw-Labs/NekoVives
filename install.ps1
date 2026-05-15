#Requires -Version 5.1
<#
.SYNOPSIS
    Installs Trader Claw on Windows.
.DESCRIPTION
    Downloads the latest release from GitHub and installs it to
    $env:LOCALAPPDATA\trader-claw\, then adds it to the user PATH.
.EXAMPLE
    irm https://raw.githubusercontent.com/Trader-Claw-Labs/Trader-Claw/main/install.ps1 | iex
#>

$ErrorActionPreference = 'Stop'

$Repo      = 'Trader-Claw-Labs/Trader-Claw'
$BinName   = 'trader-claw'
$InstallDir = Join-Path $env:LOCALAPPDATA 'trader-claw'

function Write-Info  { param($Msg) Write-Host "  [*] $Msg" -ForegroundColor Cyan }
function Write-Ok    { param($Msg) Write-Host "  [+] $Msg" -ForegroundColor Green }
function Write-Err   { param($Msg) Write-Host "  [!] $Msg" -ForegroundColor Red; exit 1 }

# Fetch latest version
Write-Info 'Fetching latest release...'
try {
    $Release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $Release.tag_name
} catch {
    Write-Err "Could not fetch release info: $_"
}

if (-not $Version) { Write-Err 'Could not determine latest version.' }
Write-Info "Latest version: $Version"

# Check if already up to date
$ExistingBin = Join-Path $InstallDir "$BinName.exe"
if (Test-Path $ExistingBin) {
    try {
        $CurrentVersion = & $ExistingBin --version 2>$null | ForEach-Object { ($_ -split ' ')[1] }
        if ("v$CurrentVersion" -eq $Version) {
            Write-Ok "$BinName $Version is already installed and up-to-date."
            exit 0
        }
        Write-Info "Updating from $CurrentVersion to $Version..."
    } catch { }
}

# Build download URL
$Artifact = "$BinName-windows-x86_64.zip"
$Url      = "https://github.com/$Repo/releases/download/$Version/$Artifact"

# Download to temp directory
$TmpDir  = Join-Path $env:TEMP "trader-claw-install-$([System.IO.Path]::GetRandomFileName())"
New-Item -ItemType Directory -Path $TmpDir | Out-Null
$ZipPath = Join-Path $TmpDir $Artifact

Write-Info "Downloading $Artifact..."
try {
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -UseBasicParsing
} catch {
    Remove-Item $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Err "Download failed: $_"
}

# Verify checksum if available
try {
    $ChecksumUrl  = "https://github.com/$Repo/releases/download/$Version/checksums.txt"
    $ChecksumFile = Join-Path $TmpDir 'checksums.txt'
    Invoke-WebRequest -Uri $ChecksumUrl -OutFile $ChecksumFile -UseBasicParsing -ErrorAction Stop

    $Expected = (Get-Content $ChecksumFile | Where-Object { $_ -match $Artifact }) -split '\s+' | Select-Object -First 1
    $Actual   = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLower()

    if ($Expected -and $Expected -ne $Actual) {
        Remove-Item $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
        Write-Err 'Checksum mismatch! Download may be corrupted.'
    }
    Write-Ok 'Checksum verified.'
} catch {
    Write-Info 'Checksum file not available, skipping verification.'
}

# Extract
Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force

# Install
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir | Out-Null
}
Copy-Item (Join-Path $TmpDir "$BinName.exe") (Join-Path $InstallDir "$BinName.exe") -Force
Remove-Item $TmpDir -Recurse -Force -ErrorAction SilentlyContinue

# Add to PATH (user scope)
$UserPath = [System.Environment]::GetEnvironmentVariable('PATH', 'User')
if ($UserPath -notlike "*$InstallDir*") {
    Write-Info "Adding $InstallDir to user PATH..."
    [System.Environment]::SetEnvironmentVariable('PATH', "$UserPath;$InstallDir", 'User')
    $env:PATH = "$env:PATH;$InstallDir"
    Write-Ok 'PATH updated. Restart your terminal for changes to take effect.'
}

Write-Ok "Trader Claw $Version installed to $InstallDir\$BinName.exe"
Write-Ok 'Run: trader-claw gateway'
