[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [string]$BuildDirectory,
    [string]$ClaudeExecutable,
    [string]$CodexExecutable,
    [string]$NodeExecutable,
    [string]$CodeGraphScript,
    [string]$ClaudeCredentials,
    [string]$CodexAuth
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
if (-not $BuildDirectory) {
    $BuildDirectory = Join-Path $repository 'target\release'
}
if (-not $NodeExecutable) {
    $NodeExecutable = (Get-Command node -ErrorAction Stop).Source
}
if (-not $CodeGraphScript) {
    $CodeGraphScript = Join-Path $env:APPDATA 'npm\node_modules\@colbymchenry\codegraph\npm-shim.js'
}
if (-not $ClaudeExecutable) {
    $nativeClaude = Join-Path $env:APPDATA 'npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe'
    $ClaudeExecutable = if (Test-Path -LiteralPath $nativeClaude -PathType Leaf) {
        $nativeClaude
    } else {
        (Get-Command claude -ErrorAction Stop).Source
    }
}
if (-not $CodexExecutable) {
    $userCodex = Join-Path $env:USERPROFILE '.codex\plugins\.plugin-appserver\codex.exe'
    $codexPackage = Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue |
        Sort-Object Version -Descending |
        Select-Object -First 1
    $nativeCodex = if ($codexPackage) {
        Join-Path $codexPackage.InstallLocation 'app\resources\codex.exe'
    } else {
        $null
    }
    $CodexExecutable = if (Test-Path -LiteralPath $userCodex -PathType Leaf) {
        $userCodex
    } elseif ($nativeCodex -and (Test-Path -LiteralPath $nativeCodex -PathType Leaf)) {
        $nativeCodex
    } else {
        (Get-Command codex -ErrorAction Stop).Source
    }
}
if (-not $ClaudeCredentials) {
    $ClaudeCredentials = Join-Path $env:USERPROFILE '.claude\.credentials.json'
}
if (-not $CodexAuth) {
    $CodexAuth = Join-Path $env:USERPROFILE '.codex\auth.json'
}

$paths = [ordered]@{
    launcher = Join-Path $BuildDirectory 'brain-benchmark-launcher.exe'
    service = Join-Path $BuildDirectory 'brain-service.exe'
    hook = Join-Path $BuildDirectory 'brain-hook.exe'
    mcp = Join-Path $BuildDirectory 'brain-mcp.exe'
    claude = $ClaudeExecutable
    codex = $CodexExecutable
    node = $NodeExecutable
    codegraph = $CodeGraphScript
    claudeCredentials = $ClaudeCredentials
    codexAuth = $CodexAuth
}
foreach ($key in @($paths.Keys)) {
    if (-not (Test-Path -LiteralPath $paths[$key] -PathType Leaf)) {
        throw "Missing $key file: $($paths[$key])"
    }
    $paths[$key] = (Resolve-Path -LiteralPath $paths[$key]).Path
}

$example = Join-Path $repository 'benchmarks\second-brain\v2\execution-templates.example.json'
$template = Get-Content -Raw -LiteralPath $example | ConvertFrom-Json
$template.brain_service_program = $paths.service
$template.launcher_environment.BENCHMARK_CLAUDE_EXECUTABLE = $paths.claude
$template.launcher_environment.BENCHMARK_CODEX_EXECUTABLE = $paths.codex
$template.launcher_environment.BENCHMARK_CODEGRAPH_NODE = $paths.node
$template.launcher_environment.BENCHMARK_CODEGRAPH_SCRIPT = $paths.codegraph
$template.launcher_environment.BENCHMARK_BRAIN_HOOK = $paths.hook
$template.launcher_environment.BENCHMARK_BRAIN_MCP = $paths.mcp
$template.launcher_environment.BENCHMARK_CLAUDE_CREDENTIALS = $paths.claudeCredentials
$template.launcher_environment.BENCHMARK_CODEX_AUTH = $paths.codexAuth
foreach ($harness in @($template.claude_code, $template.codex)) {
    foreach ($condition in @('c0', 'c1', 'c2', 'c3', 'c4')) {
        $harness.conditions.$condition.program = $paths.launcher
    }
}

$destination = [System.IO.Path]::GetFullPath($OutputPath)
$parent = Split-Path -Parent $destination
if ($parent) {
    [System.IO.Directory]::CreateDirectory($parent) | Out-Null
}
$json = $template | ConvertTo-Json -Depth 20
[System.IO.File]::WriteAllText($destination, $json + [Environment]::NewLine)
$destination
