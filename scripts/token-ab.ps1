<#
.SYNOPSIS
    Compatibility wrapper for the audited cross-harness token-savings benchmark.

.DESCRIPTION
    The former script rewrote the live Claude settings file and measured Claude only. It has been
    retired as an experimental instrument. This wrapper performs no settings writes and delegates
    only to a run that already passed `brain benchmark preflight`.

    Omit -Execute to preview the remaining immutable matrix without launching a model session.
    Supplying -Execute is the explicit paid-session boundary.

.EXAMPLE
    pwsh -File scripts/token-ab.ps1 -Project 019f... -Run 019f...

.EXAMPLE
    pwsh -File scripts/token-ab.ps1 -Project 019f... -Run 019f... -Execute
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Project,

    [Parameter(Mandatory = $true)]
    [guid]$Run,

    [switch]$Execute,

    [string]$Brain = "$env:USERPROFILE\AgentBrain\bin\brain.exe"
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $Brain -PathType Leaf)) {
    throw "brain executable not found at '$Brain'"
}

Write-Warning 'scripts/token-ab.ps1 is a compatibility wrapper. The authoritative workflow is brain benchmark.'
$arguments = @(
    'benchmark', 'run',
    '--project', $Project,
    '--run', $Run.ToString()
)
if ($Execute) {
    $arguments += '--execute'
}

& $Brain @arguments
if ($LASTEXITCODE -ne 0) {
    throw "brain benchmark run failed with exit code $LASTEXITCODE"
}
