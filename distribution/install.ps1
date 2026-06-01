param(
    [string]$Endpoint = "",
    [string]$Token = "",
    [ValidateSet("codex", "claude", "both")]
    [string]$Agent = "both",
    [string]$InstallDir = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Install-McIntegration {
    [CmdletBinding()]
    param(
        [string]$Endpoint = "",
        [string]$Token = "",
        [ValidateSet("codex", "claude", "both")]
        [string]$Agent = "both",
        [string]$InstallDir = ""
    )

    $mcpPypiSpec = if ($env:MCP_PYPI_SPEC) { $env:MCP_PYPI_SPEC } else { "edgeplane-mcp" }
    $mcpGithubSpec = if ($env:MCP_GITHUB_SPEC) { $env:MCP_GITHUB_SPEC } else { "git+https://github.com/RyanMerlin/edgeplane-integration.git#subdirectory=edgeplane-mcp" }
    $docsUrl = if ($env:DOCS_URL) { $env:DOCS_URL } else { "https://github.com/RyanMerlin/edgeplane-integration#readme" }
    $defaultLocalEndpoint = "http://localhost:8008"
    $effectiveHome = if ($env:HOME) { $env:HOME } elseif ($HOME) { $HOME } else { [Environment]::GetFolderPath("UserProfile") }
    $effectiveInstallDir = if ($InstallDir) { $InstallDir } else { Join-Path $effectiveHome ".edgeplane" }

    if ($Endpoint -and -not ($Endpoint -match '^https?://')) {
        throw "-Endpoint must start with http:// or https://"
    }

    $effectiveEndpoint = if ($Endpoint) { $Endpoint } else { $defaultLocalEndpoint }

    function Ensure-Pipx {
        if (Get-Command pipx -ErrorAction SilentlyContinue) {
            return
        }

        Write-Host "pipx not found; installing with python -m pip --user"
        if (-not (Get-Command python -ErrorAction SilentlyContinue) -and -not (Get-Command py -ErrorAction SilentlyContinue)) {
            throw "Python is required to install pipx"
        }

        if (Get-Command py -ErrorAction SilentlyContinue) {
            & py -m pip install --user pipx
            & py -m pipx ensurepath
        }
        else {
            & python -m pip install --user pipx
            & python -m pipx ensurepath
        }

        $userPipx = Join-Path $effectiveHome ".local/bin"
        if (Test-Path $userPipx) {
            $env:Path = "$userPipx;$env:Path"
        }

        if (-not (Get-Command pipx -ErrorAction SilentlyContinue)) {
            throw "pipx is not on PATH after installation"
        }
    }

    function Install-McpBridge {
        Write-Host "Installing edgeplane-mcp (PyPI first)..."
        try {
            & pipx install --force $mcpPypiSpec
            Write-Host "Installed from PyPI spec: $mcpPypiSpec"
            return
        }
        catch {
            Write-Host "PyPI install failed. Trying GitHub fallback..."
            & pipx install --force $mcpGithubSpec
            Write-Host "Installed from GitHub fallback spec."
        }
    }

    function ConvertTo-TomlStringLiteral {
        param([string]$Value)

        $escaped = $Value.Replace('\', '\\')
        $escaped = $escaped.Replace('"', '\"')
        $escaped = $escaped.Replace("`r", '\r')
        $escaped = $escaped.Replace("`n", '\n')
        $escaped = $escaped.Replace("`t", '\t')

        return '"' + $escaped + '"'
    }

    function Write-ClaudeConfig {
        param(
            [string]$OutPath
        )

        $jsonConfig = @{
            mcpServers = @{
                edgeplane = @{
                    command = "edgeplane-mcp"
                    env = @{
                        EP_BASE_URL = $effectiveEndpoint
                        EP_AGENT_TOKEN = $Token
                    }
                }
            }
        } | ConvertTo-Json -Depth 10

        Set-Content -Path $OutPath -Value $jsonConfig -Encoding UTF8
    }

    function Write-CodexConfig {
        param(
            [string]$OutPath
        )

        $endpointToml = ConvertTo-TomlStringLiteral -Value $effectiveEndpoint
        $tokenToml = ConvertTo-TomlStringLiteral -Value $Token

        $tomlConfig = @"
[mcp_servers.edgeplane]
command = "edgeplane-mcp"
startup_timeout_sec = 45
tool_timeout_sec = 60
env = { EP_BASE_URL = $endpointToml, EP_AGENT_TOKEN = $tokenToml }
"@

        Set-Content -Path $OutPath -Value $tomlConfig -Encoding UTF8
    }

    function Invoke-DoctorChecks {
        param(
            [string]$DoctorEndpoint,
            [string]$DoctorToken
        )

        if (-not (Get-Command edgeplane-mcp -ErrorAction SilentlyContinue)) {
            throw "edgeplane-mcp not found on PATH"
        }

        Write-Host "[OK] edgeplane-mcp found"

        try {
            & edgeplane-mcp --help | Out-Null
            Write-Host "[OK] edgeplane-mcp --help"
        }
        catch {
            Write-Warning "[WARN] edgeplane-mcp exists but --help failed"
        }

        if (-not $DoctorEndpoint) {
            Write-Host "[INFO] No endpoint set. Local bootstrap is complete; set EP_BASE_URL to connect."
            return
        }

        if ($DoctorEndpoint -notmatch '^https?://') {
            Write-Warning "[WARN] Endpoint does not start with http:// or https:// : $DoctorEndpoint"
            return
        }

        try {
            Invoke-WebRequest -UseBasicParsing -Method GET -Uri "$DoctorEndpoint/" -TimeoutSec 8 | Out-Null
            Write-Host "[OK] endpoint reachable: $DoctorEndpoint"
        }
        catch {
            Write-Warning "[WARN] endpoint not reachable: $DoctorEndpoint"
        }

        if ($DoctorToken) {
            try {
                $headers = @{ Authorization = "Bearer $DoctorToken" }
                Invoke-WebRequest -UseBasicParsing -Method GET -Uri "$DoctorEndpoint/mcp/health" -Headers $headers -TimeoutSec 8 | Out-Null
                Write-Host "[OK] authenticated /mcp/health"
            }
            catch {
                Write-Warning "[WARN] /mcp/health check failed (token invalid, auth policy, or connectivity)."
            }
        }
        else {
            Write-Host "[INFO] No token provided; skipping authenticated /mcp/health check."
        }
    }

    Ensure-Pipx
    Install-McpBridge

    if (-not (Get-Command edgeplane-mcp -ErrorAction SilentlyContinue)) {
        throw "edgeplane-mcp not found on PATH after install"
    }

    $envFile = Join-Path $effectiveHome ".edgeplane-agent.env"
    @(
        "EP_BASE_URL=$effectiveEndpoint"
        "EP_AGENT_TOKEN=$Token"
    ) | Set-Content -Path $envFile -Encoding UTF8

    $scriptRoot = if ($PSScriptRoot) {
        $PSScriptRoot
    }
    elseif ($PSCommandPath) {
        Split-Path -Parent $PSCommandPath
    }
    else {
        ""
    }

    $configDir = Join-Path $effectiveInstallDir "config"

    New-Item -ItemType Directory -Force -Path $configDir | Out-Null

    if ($Agent -eq "codex" -or $Agent -eq "both") {
        Write-CodexConfig -OutPath (Join-Path $configDir "codex.mcp.toml")
        Write-Host "wrote $configDir/codex.mcp.toml"
    }

    if ($Agent -eq "claude" -or $Agent -eq "both") {
        Write-ClaudeConfig -OutPath (Join-Path $configDir "claude.mcp.json")
        Write-Host "wrote $configDir/claude.mcp.json"
    }

    $doctorScript = if ($scriptRoot) { Join-Path $scriptRoot "scripts/doctor.ps1" } else { "" }
    if ($doctorScript -and (Test-Path $doctorScript)) {
        try {
            & $doctorScript -Endpoint $effectiveEndpoint -Token $Token
        }
        catch {
            Write-Warning "doctor.ps1 returned a warning/failure: $($_.Exception.Message)"
        }
    }
    else {
        try {
            Invoke-DoctorChecks -DoctorEndpoint $effectiveEndpoint -DoctorToken $Token
            Write-Host "[DONE] doctor checks finished"
        }
        catch {
            Write-Warning "inline doctor checks returned a warning/failure: $($_.Exception.Message)"
        }
    }

    Write-Host ""
    Write-Host "Installation complete."
    Write-Host ""
    Write-Host "Next steps:"
    Write-Host "1) Env file: $envFile"
    if ($doctorScript) {
        Write-Host "2) Run doctor: pwsh $doctorScript"
    }
    else {
        Write-Host "2) Run doctor: edgeplane-mcp --help"
    }
    Write-Host "3) Add MCP config in your agent from:"
    Write-Host "   - $configDir/codex.mcp.toml"
    Write-Host "   - $configDir/claude.mcp.json"
    Write-Host ""
    Write-Host "Auth/connect guidance:"
    Write-Host "- Default endpoint is localhost ($defaultLocalEndpoint)."
    Write-Host "- To use hosted Edgeplane, update endpoint/token in $envFile and rerun doctor."
    Write-Host "- Docs: $docsUrl"
}

Install-McIntegration -Endpoint $Endpoint -Token $Token -Agent $Agent -InstallDir $InstallDir
