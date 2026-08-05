#Requires -Version 5.1
<#
.SYNOPSIS
    Build the brain from source and replace the installed binaries.

.DESCRIPTION
    The binaries the system runs live in <brain-home>\bin, not in target\release. Nothing
    copies them there automatically, so a source change that is built but never installed
    leaves every reader — the dashboard, the Claude hook, the Codex MCP server — talking to
    older code while still looking healthy. This script is the step that closes that gap, and
    the post-commit hook runs it so it is not something anyone has to remember.

    Fail-safe by design. The build runs to completion before anything is replaced, so a commit
    that does not compile leaves the previous deployment live and running. The failure is
    recorded in the manifest instead, and the dashboard surfaces it.

    Every run writes <brain-home>\runtime\deploy.json describing what happened, including the
    SHA-256 of each installed binary. The dashboard verifies the files on disk against those
    hashes, which is what turns a half-applied deploy from an invisible problem into a red
    panel.

.PARAMETER Trigger
    Recorded in the manifest so a hook-driven deploy is distinguishable from a manual one.

.PARAMETER NoRestart
    Replace the binaries but leave the service stopped. Only useful when the caller intends to
    start it itself.

.EXAMPLE
    .\scripts\deploy.ps1
#>
[CmdletBinding()]
param(
    [string]$SourceRoot,
    [string]$BrainHome  = (Join-Path $env:USERPROFILE 'AgentBrain'),
    [string]$Trigger    = 'manual',
    [switch]$NoRestart
)

$ErrorActionPreference = 'Stop'

# Resolved here rather than as a param default: under `powershell -File`, which is how the
# post-commit hook invokes this, $PSScriptRoot is still empty while param defaults are being
# evaluated. Deriving it in the body is the difference between a deploy that runs and one that
# dies before it writes a manifest anyone could look at.
if (-not $SourceRoot) {
    $scriptDir = $PSScriptRoot
    if (-not $scriptDir) { $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path }
    $SourceRoot = Split-Path -Parent $scriptDir
}

$BINARIES     = @('brain.exe', 'brain-service.exe', 'brain-hook.exe', 'brain-mcp.exe')
$SERVICE_TASK = 'AgentBrain.Service'

$runtimeDir  = Join-Path $BrainHome 'runtime'
$binDir      = Join-Path $BrainHome 'bin'
$logDir      = Join-Path $runtimeDir 'logs'
$manifestPath = Join-Path $runtimeDir 'deploy.json'
$lockPath    = Join-Path $runtimeDir 'deploy.lock'

foreach ($dir in @($runtimeDir, $binDir, $logDir)) {
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
}

$stamp   = (Get-Date).ToUniversalTime().ToString('yyyyMMdd-HHmmss')
$logPath = Join-Path $logDir "deploy-$stamp.log"

function Write-Log([string]$Message) {
    $line = "[{0}] {1}" -f (Get-Date).ToUniversalTime().ToString('HH:mm:ss'), $Message
    Write-Host $line
    Add-Content -LiteralPath $logPath -Value $line -Encoding utf8
}

function Get-Utc { (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ') }

# serde_json rejects a byte-order mark, and PowerShell's utf8 encoding emits one. Write the
# bytes explicitly so the manifest is readable by the Rust side.
function Save-Manifest([hashtable]$Manifest) {
    $json = $Manifest | ConvertTo-Json -Depth 6
    [System.IO.File]::WriteAllText($manifestPath, $json, (New-Object System.Text.UTF8Encoding $false))
}

function Get-Sha256([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    try { return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
    catch { return $null }
}

# Windows refuses to overwrite a running image but permits renaming one: the process keeps its
# handle to the renamed file and releases it on exit. That is what lets brain-mcp.exe be
# replaced while Codex still holds it open, instead of silently skipping it every deploy.
function Install-Binary([string]$Source, [string]$Destination) {
    try {
        Copy-Item -LiteralPath $Source -Destination $Destination -Force -ErrorAction Stop
        return $true
    } catch {
        if (-not (Test-Path -LiteralPath $Destination)) { return $false }
    }
    $retired = "$Destination.old-$stamp"
    try {
        Move-Item -LiteralPath $Destination -Destination $retired -Force -ErrorAction Stop
        Copy-Item -LiteralPath $Source -Destination $Destination -Force -ErrorAction Stop
        Write-Log "  (was in use; previous image retired as $(Split-Path -Leaf $retired))"
        return $true
    } catch {
        # Put it back rather than leaving the install without a binary at all.
        if ((Test-Path -LiteralPath $retired) -and -not (Test-Path -LiteralPath $Destination)) {
            Move-Item -LiteralPath $retired -Destination $Destination -Force -ErrorAction SilentlyContinue
        }
        return $false
    }
}

# Retired images linger until the process holding them exits. Clear whatever has since been
# released so they do not accumulate.
function Remove-RetiredImages {
    Get-ChildItem -LiteralPath $binDir -Filter '*.old-*' -ErrorAction SilentlyContinue |
        ForEach-Object {
            try { Remove-Item -LiteralPath $_.FullName -Force -ErrorAction Stop }
            catch { }
        }
}

# One deploy at a time. An exclusive handle is self-releasing: if this process dies the lock
# dies with it, so a crashed deploy cannot wedge every future one.
try {
    $lock = [System.IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None')
} catch {
    Write-Host 'A deploy is already in progress; skipping this one.'
    exit 0
}

try {
    Write-Log "deploy start  trigger=$Trigger  source=$SourceRoot"
    Remove-RetiredImages

    $commit = $null
    $branch = $null
    $dirty  = $false
    try {
        Push-Location $SourceRoot
        $commit = (& git rev-parse HEAD).Trim()
        $branch = (& git rev-parse --abbrev-ref HEAD).Trim()
        # A dirty tree means the binaries will contain code that is in no commit, so the
        # recorded commit only approximates what gets installed. Record that rather than let
        # the dashboard imply a precision it does not have.
        $dirty = [bool](& git status --porcelain)
    } catch {
        Write-Log 'warning: could not read git state; deploying anyway'
    } finally {
        Pop-Location -ErrorAction SilentlyContinue
    }
    Write-Log "commit=$commit branch=$branch dirty=$dirty"

    $manifest = @{
        schema_version    = 1
        status            = 'running'
        commit            = $commit
        branch            = $branch
        source_root       = $SourceRoot
        dirty             = $dirty
        source_fingerprint = $null
        trigger           = $Trigger
        started_at        = (Get-Utc)
        finished_at       = $null
        binaries          = @()
        service_restarted = $false
        error             = $null
        log               = $logPath
    }
    Save-Manifest $manifest

    # --- Build ------------------------------------------------------------------
    # Nothing is replaced until this succeeds. cmd owns the redirection so PowerShell does not
    # wrap cargo's stderr into error records and mistake a warning for a failure.
    Write-Log 'building release binaries'
    Push-Location $SourceRoot
    $buildCommand = 'cargo build --release --bin brain --bin brain-service --bin brain-hook --bin brain-mcp 2>&1'
    $buildOutput  = & cmd /c $buildCommand
    $buildExit    = $LASTEXITCODE
    Pop-Location

    $buildOutput | ForEach-Object { Add-Content -LiteralPath $logPath -Value $_ -Encoding utf8 }

    if ($buildExit -ne 0) {
        $errorLines = @($buildOutput | Where-Object { $_ -match '^error' } | Select-Object -First 8)
        if ($errorLines.Count -eq 0) { $errorLines = @($buildOutput | Select-Object -Last 8) }
        $manifest.status      = 'failed'
        $manifest.error       = ($errorLines -join "`n")
        $manifest.finished_at = (Get-Utc)
        Save-Manifest $manifest
        Write-Log "BUILD FAILED (exit $buildExit) - installed binaries left untouched"
        Write-Log $manifest.error
        exit 1
    }
    Write-Log 'build ok'

    # Fingerprint the build inputs using the binary just built, so the dashboard can tell
    # whether rebuilding would differ without either side reimplementing the walk. Recorded
    # before installing: it describes the source these artifacts came from.
    $releaseDir  = Join-Path $SourceRoot 'target\release'
    $fingerprint = $null
    try {
        $fingerprint = (& (Join-Path $releaseDir 'brain.exe') source-fingerprint --source-root $SourceRoot).Trim()
        Write-Log "source fingerprint $($fingerprint.Substring(0,12))"
    } catch {
        Write-Log 'warning: could not fingerprint build inputs; drift will fall back to commits'
    }
    $manifest.source_fingerprint = $fingerprint

    # --- Install ----------------------------------------------------------------
    $serviceWasRunning = [bool](Get-Process brain-service -ErrorAction SilentlyContinue)
    if ($serviceWasRunning) {
        Write-Log 'stopping service'
        & schtasks /End /TN $SERVICE_TASK | Out-Null
    }

    $installed  = @()
    $skipped    = @()
    foreach ($name in $BINARIES) {
        $source = Join-Path $releaseDir $name
        if (-not (Test-Path -LiteralPath $source)) {
            Write-Log "  $name : NOT BUILT"
            $skipped += $name
            $installed += @{ name = $name; sha256 = $null; replaced = $false }
            continue
        }
        $target = Join-Path $binDir $name
        if (Install-Binary -Source $source -Destination $target) {
            $hash = Get-Sha256 $target
            Write-Log "  $name : installed $($hash.Substring(0,12))"
            $installed += @{ name = $name; sha256 = $hash; replaced = $true }
        } else {
            Write-Log "  $name : LOCKED - could not replace"
            $skipped += $name
            $installed += @{ name = $name; sha256 = (Get-Sha256 $target); replaced = $false }
        }
    }

    # --- Restart ----------------------------------------------------------------
    $restarted = $false
    if (-not $NoRestart) {
        Write-Log 'starting service'
        & schtasks /Run /TN $SERVICE_TASK | Out-Null
        $deadline = (Get-Date).AddSeconds(15)
        while ((Get-Date) -lt $deadline) {
            if (Get-Process brain-service -ErrorAction SilentlyContinue) { $restarted = $true; break }
            Start-Sleep -Milliseconds 500
        }
        if ($restarted) { Write-Log 'service running' } else { Write-Log 'WARNING: service did not come back up' }
    }

    $manifest.status            = 'succeeded'
    $manifest.binaries          = $installed
    $manifest.service_restarted = $restarted
    $manifest.finished_at       = (Get-Utc)
    Save-Manifest $manifest

    if ($skipped.Count -gt 0) {
        Write-Log "deploy finished with $($skipped.Count) binary/binaries not replaced: $($skipped -join ', ')"
        exit 2
    }
    Write-Log 'deploy finished cleanly'
    exit 0
}
catch {
    $message = $_.Exception.Message
    Write-Log "DEPLOY ERROR: $message"
    try {
        $manifest.status      = 'failed'
        $manifest.error       = $message
        $manifest.finished_at = (Get-Utc)
        Save-Manifest $manifest
    } catch { }
    exit 1
}
finally {
    $lock.Close()
    $lock.Dispose()
}
