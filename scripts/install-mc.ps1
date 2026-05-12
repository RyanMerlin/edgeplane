#Requires -Version 5.1
<#
.SYNOPSIS
    Install the mc CLI on Windows.
.DESCRIPTION
    Downloads the prebuilt mc binary from GitHub Releases, verifies its SHA256
    checksum, and installs it to $env:USERPROFILE\.local\bin\mc.exe.
    Falls back to building from source if the release asset is unavailable.
.PARAMETER InstallDir
    Directory to install mc.exe into. Defaults to $env:USERPROFILE\.local\bin.
.PARAMETER BaseUrl
    Override the GitHub Releases base URL (useful for testing).
#>
param(
    [string]$InstallDir = "$env:USERPROFILE\.local\bin",
    [string]$BaseUrl = "https://github.com/RyanMerlin/missioncontrol/releases/latest/download"
)

$ErrorActionPreference = 'Stop'

$arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($arch -ne [System.Runtime.InteropServices.Architecture]::X64) {
    Write-Warning "Only x86_64 Windows is supported for prebuilt binaries. Got: $arch"
    Write-Warning "Please build from source: cd integrations\mc && cargo build --release"
    exit 1
}

$artifact  = "mc-windows-x86_64.exe"
$targetExe = Join-Path $InstallDir "mc.exe"
$tmpExe    = "$targetExe.tmp"
$tmpChecks = "$targetExe.checksums.tmp"

# Ensure install dir exists
if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

function Remove-Temps {
    Remove-Item -Force -ErrorAction SilentlyContinue $tmpExe, $tmpChecks
}

Write-Host "Downloading $artifact from $BaseUrl ..."
try {
    Invoke-WebRequest -Uri "$BaseUrl/$artifact" -OutFile $tmpExe -UseBasicParsing -TimeoutSec 60
} catch {
    Write-Warning "Binary download failed: $_"
    Write-Warning "Falling back to source build (requires cargo / Rust toolchain)."
    Remove-Temps
    $repoRoot = Split-Path $PSScriptRoot -Parent
    Push-Location (Join-Path $repoRoot "integrations\mc")
    cargo build --release
    Pop-Location
    Copy-Item (Join-Path $repoRoot "integrations\mc\target\release\mc.exe") $targetExe -Force
    Write-Host "Installed mc from source to $targetExe"
    exit 0
}

# Verify checksum
try {
    Invoke-WebRequest -Uri "$BaseUrl/checksums.txt" -OutFile $tmpChecks -UseBasicParsing -TimeoutSec 15
    $checksumLine = Get-Content $tmpChecks | Where-Object { $_ -match "\s$([regex]::Escape($artifact))$" }
    if ($checksumLine) {
        $expected = ($checksumLine -split '\s+')[0].ToLower()
        $actual   = (Get-FileHash -Path $tmpExe -Algorithm SHA256).Hash.ToLower()
        if ($expected -ne $actual) {
            Write-Error "SHA256 mismatch! Expected: $expected  Got: $actual"
            Remove-Temps
            exit 1
        }
        Write-Host "Checksum verified."
    } else {
        Write-Warning "Artifact not found in checksums.txt — skipping verification."
    }
} catch {
    Write-Warning "Could not download checksums.txt — skipping verification. ($_)"
}

Move-Item -Force $tmpExe $targetExe
Remove-Temps

Write-Host "Installed mc to $targetExe"

# PATH hint
$userPath = [System.Environment]::GetEnvironmentVariable("PATH", "User")
if ($userPath -notlike "*$InstallDir*") {
    Write-Host ""
    Write-Host "Add mc to your PATH by running:"
    Write-Host "  [System.Environment]::SetEnvironmentVariable('PATH', '$InstallDir;' + [System.Environment]::GetEnvironmentVariable('PATH','User'), 'User')"
    Write-Host "Then restart your terminal."
}

& $targetExe --version
Write-Host ""
Write-Host "Launch an agent:"
Write-Host "  `$env:MC_TOKEN='<token>'; `$env:MC_BASE_URL='https://your-mc.example.com'; mc launch codex"
