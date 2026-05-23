#Requires -Version 5.1
<#
.SYNOPSIS
    Bootstrap the edgeplane CLI on Windows.
.DESCRIPTION
    Thin wrapper around install-edgeplane.ps1 so Windows users have a PowerShell-native
    bootstrap entrypoint that mirrors bootstrap-edgeplane.sh on Unix shells.
.PARAMETER InstallDir
    Directory to install edgeplane.exe into. Defaults to $env:USERPROFILE\.local\bin.
.PARAMETER BaseUrl
    Override the GitHub Releases base URL (useful for testing).
#>
param(
    [string]$InstallDir = "$env:USERPROFILE\.local\bin",
    [string]$BaseUrl = "https://github.com/RyanMerlin/edgeplane/releases/latest/download"
)

$ErrorActionPreference = 'Stop'

if ($PSScriptRoot) {
    $installScript = Join-Path $PSScriptRoot "install-edgeplane.ps1"
}
else {
    $installScript = ""
}

if ($installScript -and (Test-Path $installScript)) {
    & $installScript -InstallDir $InstallDir -BaseUrl $BaseUrl
    exit $LASTEXITCODE
}

$rawInstallUrl = "https://raw.githubusercontent.com/RyanMerlin/edgeplane/main/scripts/install-edgeplane.ps1"
$tempInstallScript = Join-Path ([System.IO.Path]::GetTempPath()) ("install-edgeplane-" + [System.Guid]::NewGuid().ToString("N") + ".ps1")

try {
    Invoke-WebRequest -Uri $rawInstallUrl -OutFile $tempInstallScript -UseBasicParsing -TimeoutSec 60
    & $tempInstallScript -InstallDir $InstallDir -BaseUrl $BaseUrl
    exit $LASTEXITCODE
}
finally {
    Remove-Item -LiteralPath $tempInstallScript -Force -ErrorAction SilentlyContinue
}
