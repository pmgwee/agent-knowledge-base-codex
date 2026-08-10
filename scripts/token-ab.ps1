<#
.SYNOPSIS
    Matched-pair A/B: what a task costs with the brain, and what it costs without.

.DESCRIPTION
    The one claim this project has never been able to back. Half the fraction exists — a mean of
    ~1,115 tokens delivered per orientation, counted at receipt since 407d345. The other half, what
    a session would have spent *without* it, has never been captured, so there has never been a
    percentage to quote.

    This runs it. Five tasks from this repository's own history, each with an answer that lives in
    the brain and is expensive to re-derive from code; three conditions; three repeats; forty-five
    headless sessions.

    This line said "two conditions; thirty sessions" for as long as the third condition below has
    existed, which understated the cost by a third — in a script whose whole posture is that
    spending money is the operator's deliberate call. The count is computed at the matrix line;
    trust that over any prose here, including this.

    THE CONFOUND THIS CONTROLS FOR, WHICH THE ORIGINAL DESIGN DID NOT
    ------------------------------------------------------------------
    CodeGraph also sits on UserPromptSubmit, in the same settings file, firing on every prompt
    alongside the brain hook. A naive cold/warm split therefore measures *brain + CodeGraph* against
    *CodeGraph*, and attributes whatever CodeGraph saves to the brain. Worse, CodeGraph's own
    published claim is ~35% less cost — the same order as anything the brain could show — so the
    two are not separable after the fact.

    So there are three conditions, not two, and the third is what makes the number mean anything:

      bare   — neither hook. The floor: an agent with the repo and nothing else.
      code   — CodeGraph only. What structural indexing alone is worth here.
      warm   — both. What the machine actually does today.

    The brain's contribution is (code - warm), not (bare - warm). Reporting the latter as the
    brain's saving would be the same error as quoting our 90.0% on one category against someone
    else's pooled 95.2%.

    WHAT THIS COSTS — AND THE THREE COSTS THAT ARE NOT MONEY
    --------------------------------------------------------
    Forty-five headless agent sessions against a real repository. That is real money and it is the
    operator's call, which is why nothing here runs on a schedule and why the dry run is the default
    posture: run it once, deliberately, when you mean to. Every run records `total_cost_usd`, so
    after the first execution this stops being an estimate.

    The money is the smallest of it:

      * These sessions are captured. Capture is transcript-based and watches ~/.claude/projects, so
        forty-five sessions become events, which become consolidation jobs. Running this while a
        backlog drains lengthens that backlog.
      * `claude -p` draws on the same provider quota as the consolidation worker. Measured
        precedent: a synthesis batch pushed the drain from 75 jobs/hour to 39 while it ran.
      * The token number alone means nothing without the blind grading described at the foot of this
        script. A warm session that answers badly and stops early wins on tokens and loses on the
        only thing being claimed. Budget that grading before spending the sessions.

    PRECONDITION THE DESIGN STATES
    ------------------------------
    A half-consolidated brain understates the warm condition. Check `brain digest` first; with
    thousands of jobs still queued behind provider quota, the warm side is measuring a brain that
    has not finished thinking about its own evidence.

    KNOWN CONFOUND, NOT YET FIXED
    -----------------------------
    Conditions run in a fixed order — every `bare` run, then every `code`, then every `warm`. Any
    drift across the run (quota throttling, machine load, cache state) is therefore confounded with
    condition, and `warm` always runs last. Interleaving the matrix would cost nothing and remove
    it; it is called out here rather than silently fixed because doing so changes what a comparison
    against an earlier run means.

.PARAMETER Repeats
    Runs per task per condition. The design says 3; fewer is a smoke test, not a measurement.

.PARAMETER Out
    Directory for per-run JSON and the summary.

.PARAMETER Conditions
    Subset of bare, code, warm. Default all three.

.PARAMETER Execute
    Actually spawn sessions. Without it this prints the matrix and exits, which is the safe default
    for a script that spends money.
#>
[CmdletBinding()]
param(
    [int]$Repeats = 3,
    [string]$Out = "$env:USERPROFILE\AgentBrain\runtime\token-ab",
    [string[]]$Conditions = @('bare', 'code', 'warm'),
    [switch]$Execute
)

$ErrorActionPreference = 'Stop'

# Five tasks from this repository's real history. Each has a known-correct answer that lives in the
# brain and costs real reading to re-derive from source. Fixed before running, in the file, in git —
# so nobody can say the set was chosen after seeing the numbers.
$tasks = @(
    @{
        id     = 'crt-static'
        prompt = 'I want to delete .cargo/config.toml to simplify the build. Is that safe? Answer in three sentences.'
        rubric = 'Names +crt-static, and that removal fails only in restricted launch contexts (Codex sandbox, Task Scheduler session 0) while a normal cargo build still appears to work.'
    },
    @{
        id     = 'snapshot-mirror'
        prompt = 'I am adding a field to the dashboard snapshot struct. What else must change? Answer in three sentences.'
        rubric = 'Names lib/snapshot-types.ts in the separate agent-brain-dashboard repo, and that nothing enforces the correspondence.'
    },
    @{
        id     = 'register-restart'
        prompt = 'I just ran brain register on a new project. What is the step people miss? Answer in two sentences.'
        rubric = 'Names restarting AgentBrain.Service, because capture bindings are built once at startup.'
    },
    @{
        id     = 'retrieval-empty'
        prompt = 'A search that should match returns nothing. Where has this bitten this codebase before? Answer in three sentences.'
        rubric = 'Names the id-restriction/keyword-selector interaction, or the AND-joined FTS terms, as a past defect that read as working code.'
    },
    @{
        id     = 'huge-job'
        prompt = 'A consolidation job came out enormous. What bounds it? Answer in two sentences.'
        rubric = 'Names both bounds — 200 events AND 400 KB — and that size counts payload and raw.'
    }
)

$settings = Join-Path $env:USERPROFILE '.claude\settings.json'
$backup = Join-Path $Out 'settings.backup.json'
New-Item -ItemType Directory -Force -Path $Out | Out-Null

function Get-HookCommands($document) {
    $names = @()
    foreach ($event in $document.hooks.PSObject.Properties.Name) {
        foreach ($group in $document.hooks.$event) {
            foreach ($hook in $group.hooks) { $names += $hook.command }
        }
    }
    return $names
}

# Condition is applied by rewriting the hook file and restoring it afterwards. The alternative —
# `brain uninstall-hooks` — is a heavier, less reversible operation on a live install, and it would
# leave the machine broken if this script died mid-run.
function Set-Condition($condition) {
    $document = Get-Content $settings -Raw | ConvertFrom-Json
    foreach ($event in $document.hooks.PSObject.Properties.Name) {
        foreach ($group in $document.hooks.$event) {
            $group.hooks = @($group.hooks | Where-Object {
                $isBrain = $_.command -match 'brain-hook'
                $isCode = $_.command -match 'codegraph'
                switch ($condition) {
                    'bare' { -not $isBrain -and -not $isCode }
                    'code' { -not $isBrain }
                    'warm' { $true }
                    default { $true }
                }
            })
        }
    }
    $document | ConvertTo-Json -Depth 20 | Out-File $settings -Encoding utf8
}

Write-Host "matrix: $($tasks.Count) tasks x $($Conditions.Count) conditions x $Repeats repeats = $($tasks.Count * $Conditions.Count * $Repeats) sessions"
Write-Host "conditions: $($Conditions -join ', ')"
Write-Host "out: $Out"
if (-not $Execute) {
    Write-Host ''
    Write-Host 'Dry run. Nothing was spawned and no settings were touched.'
    Write-Host 'Re-run with -Execute when you mean to spend the tokens.'
    foreach ($t in $tasks) { Write-Host ("  {0,-18} {1}" -f $t.id, $t.prompt) }
    exit 0
}

Copy-Item $settings $backup -Force
Write-Host "settings backed up to $backup"

$results = @()
try {
    foreach ($condition in $Conditions) {
        Set-Condition $condition
        foreach ($task in $tasks) {
            for ($run = 1; $run -le $Repeats; $run++) {
                $label = "$($task.id)-$condition-$run"
                $started = Get-Date
                # --output-format json carries usage, which is the measurement. A fresh session per
                # run is the point: the orientation only ever fires at session start.
                $raw = claude -p $task.prompt --output-format json 2>&1 | Out-String
                $elapsed = (Get-Date) - $started
                $payload = $null
                try { $payload = $raw | ConvertFrom-Json } catch { }
                $record = [ordered]@{
                    task       = $task.id
                    condition  = $condition
                    run        = $run
                    seconds    = [math]::Round($elapsed.TotalSeconds, 1)
                    input      = $payload.usage.input_tokens
                    cacheRead  = $payload.usage.cache_read_input_tokens
                    output     = $payload.usage.output_tokens
                    costUsd    = $payload.total_cost_usd
                    rubric     = $task.rubric
                    answer     = $payload.result
                }
                $results += [pscustomobject]$record
                $record | ConvertTo-Json -Depth 10 | Out-File (Join-Path $Out "$label.json") -Encoding utf8
                Write-Host ("  {0,-30} {1,7} in  {2,7} cached  {3,6}s" -f $label, $record.input, $record.cacheRead, $record.seconds)
            }
        }
    }
}
finally {
    # Always restore, including on Ctrl-C. A benchmark that leaves the machine's hooks disabled is
    # worse than one that never ran.
    Copy-Item $backup $settings -Force
    Write-Host "settings restored from $backup"
}

$results | Export-Csv (Join-Path $Out 'runs.csv') -NoTypeInformation
Write-Host ''
Write-Host 'per condition — mean input tokens'
$results | Group-Object condition | ForEach-Object {
    $mean = ($_.Group | Measure-Object input -Average).Average
    Write-Host ("  {0,-6} {1,10:N0}" -f $_.Name, $mean)
}
Write-Host ''
Write-Host 'The brain''s share is (code - warm), not (bare - warm). Grade the answers against each'
Write-Host 'rubric blind before quoting any of this, and report the count of tasks where warm scored'
Write-Host 'WORSE than code with equal prominence — a memory system that misleads with stale context'
Write-Host 'is the failure mode this exists to catch.'
