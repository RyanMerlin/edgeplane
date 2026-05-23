param(
    [string]$Endpoint = "",
    [string]$Token = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Get-Command edgeplane-mcp -ErrorAction SilentlyContinue)) {
    throw "[FAIL] edgeplane-mcp not found on PATH"
}

Write-Host "[OK] edgeplane-mcp found"

try {
    & edgeplane-mcp --help | Out-Null
    Write-Host "[OK] edgeplane-mcp --help"
}
catch {
    Write-Warning "[WARN] edgeplane-mcp exists but --help failed"
}

if ($Endpoint) {
    $previousBaseUrl = $env:EP_BASE_URL
    $previousToken = $env:EP_TOKEN
    $env:EP_BASE_URL = $Endpoint
    $env:EP_TOKEN = $Token
    try {
        $doctorRaw = & edgeplane-mcp doctor | Out-String
        if (-not $doctorRaw) {
            Write-Warning "[WARN] edgeplane-mcp doctor returned no output"
        }
        else {
            $doctorJson = $doctorRaw | ConvertFrom-Json
            if ($doctorJson.checks.$Endpoint.health_ok -eq $true) {
                Write-Host "[OK] edgeplane-mcp doctor health check"
            }
            else {
                Write-Warning "[WARN] edgeplane-mcp doctor reports health check failure"
            }
            if ($Token) {
                if ($doctorJson.checks.$Endpoint.tools_ok -eq $true) {
                    Write-Host "[OK] edgeplane-mcp doctor tools check"
                }
                else {
                    Write-Warning "[WARN] edgeplane-mcp doctor reports tools check failure"
                }
            }
        }
    }
    catch {
        Write-Warning "[WARN] edgeplane-mcp doctor command failed"
    }
    finally {
        $env:EP_BASE_URL = $previousBaseUrl
        $env:EP_TOKEN = $previousToken
    }
}

if (-not $Endpoint) {
    Write-Host "[INFO] No endpoint set. Local bootstrap is complete; set EP_BASE_URL to connect."
    exit 0
}

if ($Endpoint -notmatch '^https?://') {
    Write-Warning "[WARN] Endpoint does not start with http:// or https:// : $Endpoint"
    exit 0
}

try {
    Invoke-WebRequest -UseBasicParsing -Method GET -Uri "$Endpoint/" -TimeoutSec 8 | Out-Null
    Write-Host "[OK] endpoint reachable: $Endpoint"
}
catch {
    Write-Warning "[WARN] endpoint not reachable: $Endpoint"
}

if ($Token) {
    try {
        $headers = @{ Authorization = "Bearer $Token" }
        Invoke-WebRequest -UseBasicParsing -Method GET -Uri "$Endpoint/mcp/health" -Headers $headers -TimeoutSec 8 | Out-Null
        Write-Host "[OK] authenticated /mcp/health"
    }
    catch {
        Write-Warning "[WARN] /mcp/health check failed (token invalid, auth policy, or connectivity)."
    }
}
else {
    Write-Host "[INFO] No token provided; skipping authenticated /mcp/health check."
}

if (Get-Command edgeplane-explorer -ErrorAction SilentlyContinue) {
    if ($Endpoint) {
        $previousBaseUrl = $env:EP_BASE_URL
        $previousToken = $env:EP_TOKEN
        $env:EP_BASE_URL = $Endpoint
        $env:EP_TOKEN = $Token
        try {
            $explorerRaw = & edgeplane-explorer tree --format json 2>$null | Out-String
            if (-not $explorerRaw) {
                Write-Warning "[WARN] edgeplane-explorer returned no output"
            }
            else {
                $explorerJson = $explorerRaw | ConvertFrom-Json
                if ($null -ne $explorerJson.mission_count) {
                    Write-Host "[OK] edgeplane-explorer tree --format json"
                }
                else {
                    Write-Warning "[WARN] edgeplane-explorer returned unexpected JSON shape"
                }
            }
        }
        catch {
            Write-Warning "[WARN] edgeplane-explorer failed"
        }
        finally {
            $env:EP_BASE_URL = $previousBaseUrl
            $env:EP_TOKEN = $previousToken
        }
    }
    else {
        Write-Host "[INFO] edgeplane-explorer found; skipping explorer run because endpoint is empty"
    }
}
else {
    Write-Warning "[WARN] edgeplane-explorer not found on PATH"
}

Write-Host "[DONE] doctor checks finished"
