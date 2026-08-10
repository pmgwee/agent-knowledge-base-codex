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

      * `claude -p` draws on the same provider quota as the consolidation worker. Measured
        precedent: a synthesis batch pushed the drain from 75 jobs/hour to 39 while it ran. This is
        the one that argues for waiting until a backlog has drained.
      * The token number alone means nothing without the blind grading described at the foot of this
        script. A warm session that answers badly and stops early wins on tokens and loses on the
        only thing being claimed. Budget that grading before spending the sessions.
      * What the run leaves in the ledger — see WHAT THIS RUN LEAVES BEHIND below. Not the backlog
        it adds, which is about three jobs; the answers it teaches the brain.

    PRECONDITION THE DESIGN STATES
    ------------------------------
    A half-consolidated brain understates the warm condition. Check `brain digest` first; with
    thousands of jobs still queued behind provider quota, the warm side is measuring a brain that
    has not finished thinking about its own evidence.

    ORDER — SHUFFLED, WITH A RECORDED SEED
    --------------------------------------
    Conditions used to run in a fixed block order: every `bare`, then every `code`, then every
    `warm`. Anything drifting across a forty-five-session run — quota throttling, machine load,
    cache state — was therefore confounded with condition, and `warm` always ran last, which is the
    direction that flatters the result this script exists to test.

    The matrix is now shuffled as one flat list, so condition and position are independent. The seed
    is a parameter and is written into the manifest, so a run can be reproduced exactly; leaving it
    unset draws one and records what it drew. Note that switching conditions is no longer free —
    every run now rewrites the hook file — but that is milliseconds against a session, and it buys
    the only ordering guarantee that matters.

    WHAT THIS RUN LEAVES BEHIND
    ---------------------------
    These sessions are captured, like any other. Two consequences, and only the second is worth
    engineering around:

      * Backlog growth is negligible. Measured against this project's own ledger, a session's median
        is ~15 events, so forty-five of them is ~675 — roughly three consolidation jobs at the
        200-event bound, against a backlog in the thousands. An earlier version of this note implied
        otherwise; it was wrong by two orders of magnitude.
      * Self-contamination on a re-run is the real risk. The benchmark asks five questions whose
        answers live in the brain. Once captured and consolidated, the brain has learned the
        benchmark's own answers — so a second run's `warm` condition scores better for a reason that
        has nothing to do with the memory system working.

    So every run is given an explicit `--session-id` and all forty-five are written to
    `sessions.json`. That makes this run's footprint findable in one query rather than invisible:
    retire it before re-running, or the second number is not comparable to the first.

.PARAMETER Repeats
    Runs per task per condition. The design says 3; fewer is a smoke test, not a measurement.

.PARAMETER Out
    Directory for per-run JSON and the summary.

.PARAMETER Conditions
    Subset of bare, code, warm. Default all three.

.PARAMETER Seed
    Shuffle seed. Omit to draw one; whatever is used is recorded in the manifest.

.PARAMETER SelfTest
    Exercise the condition switch against a throwaway settings fixture and exit. Spends nothing,
    touches no real settings file, and is the only cheap way to know the three conditions are
    actually three conditions.

.PARAMETER Execute
    Actually spawn sessions. Without it this prints the matrix and exits, which is the safe default
    for a script that spends money.
#>
[CmdletBinding()]
param(
    [int]$Repeats = 3,
    [string]$Out = "$env:USERPROFILE\AgentBrain\runtime\token-ab",
    [string[]]$Conditions = @('bare', 'code', 'warm'),
    [int]$Seed = 0,
    [switch]$SelfTest,
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

# Condition is applied by rewriting the hook file and restoring it afterwards. The alternative —
# `brain uninstall-hooks` — is a heavier, less reversible operation on a live install, and it would
# leave the machine broken if this script died mid-run.
#
# ALWAYS DERIVED FROM THE PRISTINE BACKUP, NEVER FROM THE LIVE FILE.
#
# It used to read $settings — the file it had itself just stripped. With the old block order that
# was silently fatal: `bare` removed both hooks and wrote the result, then `code` filtered *that*
# and kept nothing, then `warm` kept nothing again. Conditions two and three would both have
# measured `bare`, the report would have shown three near-identical columns, and the honest reading
# of that would have been "the brain saves nothing" — a conclusion about the measurement wearing the
# costume of a conclusion about the system.
#
# Interleaving makes it worse rather than better, since the file is now rewritten before every
# single run. Hence the backup as the one source of truth.
function Set-Condition($condition) {
    $document = Get-Content $backup -Raw | ConvertFrom-Json
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

# Which hooks a condition should actually leave installed, read back off disk.
#
# Asserted before the first run of each condition rather than trusted. The bug above produced a
# perfectly well-formed settings file every time; nothing about it looked wrong, and the only
# evidence would have been three suspiciously similar columns at the end of a run that had already
# been paid for.
function Assert-Condition($condition) {
    $document = Get-Content $settings -Raw | ConvertFrom-Json
    $commands = @()
    foreach ($event in $document.hooks.PSObject.Properties.Name) {
        foreach ($group in $document.hooks.$event) {
            foreach ($hook in $group.hooks) { $commands += $hook.command }
        }
    }
    $hasBrain = @($commands | Where-Object { $_ -match 'brain-hook' }).Count -gt 0
    $hasCode = @($commands | Where-Object { $_ -match 'codegraph' }).Count -gt 0
    $wantBrain = $condition -eq 'warm'
    $wantCode = ($condition -eq 'warm') -or ($condition -eq 'code')
    if ($hasBrain -ne $wantBrain) {
        throw "condition '$condition' wanted brain-hook installed=$wantBrain but found $hasBrain"
    }
    if ($hasCode -ne $wantCode) {
        throw "condition '$condition' wanted codegraph installed=$wantCode but found $hasCode"
    }
}

# Prove the condition switch actually switches, without spending a session.
#
# This exists because the bug it checks for was invisible: `Set-Condition` produced a well-formed
# settings file every time, and the only symptom would have been three suspiciously similar columns
# at the end of a run that had already been paid for. Applying the conditions in every order and
# asserting after each is a second of work; discovering it afterwards costs the whole matrix.
if ($SelfTest) {
    $fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("token-ab-selftest-" + [guid]::NewGuid())
    New-Item -ItemType Directory -Force -Path $fixtureRoot | Out-Null
    $settings = Join-Path $fixtureRoot 'settings.json'
    $backup = Join-Path $fixtureRoot 'settings.backup.json'
    @'
{
  "hooks": {
    "SessionStart": [
      { "hooks": [
        { "type": "command", "command": "C:\\Users\\me\\AgentBrain\\bin\\brain-hook.exe --harness claude-code" },
        { "type": "command", "command": "codegraph hook session-start" },
        { "type": "command", "command": "some-unrelated-hook.exe" }
      ] }
    ],
    "UserPromptSubmit": [
      { "hooks": [
        { "type": "command", "command": "C:\\Users\\me\\AgentBrain\\bin\\brain-hook.exe --harness claude-code" },
        { "type": "command", "command": "codegraph hook prompt" }
      ] }
    ]
  }
}
'@ | Out-File $settings -Encoding utf8
    Copy-Item $settings $backup -Force

    # Every order, including the one that broke it: bare first, then something that must come back.
    $failures = 0
    foreach ($order in @(@('bare', 'code', 'warm'), @('warm', 'bare', 'warm'), @('bare', 'warm', 'code', 'bare', 'warm'))) {
        foreach ($condition in $order) {
            Set-Condition $condition
            try {
                Assert-Condition $condition
                Write-Host ("  ok    {0,-6} after [{1}]" -f $condition, ($order -join ' '))
            }
            catch {
                $failures++
                Write-Host ("  FAIL  {0,-6} after [{1}] : {2}" -f $condition, ($order -join ' '), $_.Exception.Message)
            }
        }
    }
    # The unrelated hook must survive every condition; stripping a third party's hook would be a
    # different and worse bug than the one above.
    $survivors = (Get-Content $settings -Raw | ConvertFrom-Json)
    $unrelated = @($survivors.hooks.SessionStart[0].hooks | Where-Object { $_.command -match 'some-unrelated-hook' }).Count
    if ($unrelated -ne 1) {
        $failures++
        Write-Host "  FAIL  an unrelated third-party hook did not survive the condition switch"
    }
    else {
        Write-Host "  ok    an unrelated third-party hook survives every condition"
    }
    Remove-Item -Recurse -Force $fixtureRoot
    if ($failures -gt 0) {
        Write-Host "self-test: $failures failure(s)"
        exit 1
    }
    Write-Host 'self-test: passed. Conditions switch correctly in any order.'
    exit 0
}

if ($Seed -eq 0) { $Seed = Get-Random -Minimum 1 -Maximum 2147483647 }

# The matrix as one flat list, then shuffled, so condition and position are independent.
$matrix = @()
foreach ($condition in $Conditions) {
    foreach ($task in $tasks) {
        for ($run = 1; $run -le $Repeats; $run++) {
            $matrix += [pscustomobject]@{
                task      = $task.id
                condition = $condition
                run       = $run
                prompt    = $task.prompt
                rubric    = $task.rubric
                # A known session id per run, so this benchmark's footprint in the ledger can be
                # found in one query. See WHAT THIS RUN LEAVES BEHIND above: the concern is not the
                # three consolidation jobs, it is that a re-run would score against a brain that had
                # learned this benchmark's own answers.
                sessionId = [guid]::NewGuid().ToString()
            }
        }
    }
}
$matrix = $matrix | Get-Random -Count $matrix.Count -SetSeed $Seed

Write-Host "matrix: $($tasks.Count) tasks x $($Conditions.Count) conditions x $Repeats repeats = $($matrix.Count) sessions"
Write-Host "conditions: $($Conditions -join ', ')  (shuffled, seed $Seed)"
Write-Host "out: $Out"
if (-not $Execute) {
    Write-Host ''
    Write-Host 'Dry run. Nothing was spawned and no settings were touched.'
    Write-Host 'Re-run with -Execute when you mean to spend the tokens.'
    foreach ($t in $tasks) { Write-Host ("  {0,-18} {1}" -f $t.id, $t.prompt) }
    Write-Host ''
    Write-Host 'first ten of the shuffled order:'
    foreach ($entry in $matrix | Select-Object -First 10) {
        Write-Host ("  {0,-6} {1,-18} run {2}" -f $entry.condition, $entry.task, $entry.run)
    }
    exit 0
}

Copy-Item $settings $backup -Force
Write-Host "settings backed up to $backup"

# Written before the first session, not after the last: if the run dies halfway, the ids of the
# sessions that did happen are the only way to find what they left in the ledger.
[pscustomobject]@{
    seed       = $Seed
    startedUtc = (Get-Date).ToUniversalTime().ToString('o')
    sessions   = $matrix | Select-Object task, condition, run, sessionId
} | ConvertTo-Json -Depth 10 | Out-File (Join-Path $Out 'sessions.json') -Encoding utf8

$results = @()
$applied = ''
try {
    foreach ($entry in $matrix) {
        # Interleaved, so the condition changes constantly. Rewriting only on change keeps the file
        # churn down without reintroducing any coupling between order and condition.
        if ($entry.condition -ne $applied) {
            Set-Condition $entry.condition
            Assert-Condition $entry.condition
            $applied = $entry.condition
        }
        $label = "$($entry.task)-$($entry.condition)-$($entry.run)"
        $started = Get-Date
        # --output-format json carries usage, which is the measurement. A fresh session per run is
        # the point: the orientation only ever fires at session start.
        $raw = claude -p $entry.prompt --session-id $entry.sessionId --output-format json 2>&1 | Out-String
        $elapsed = (Get-Date) - $started
        $payload = $null
        try { $payload = $raw | ConvertFrom-Json } catch { }
        $record = [ordered]@{
            task      = $entry.task
            condition = $entry.condition
            run       = $entry.run
            sessionId = $entry.sessionId
            seconds   = [math]::Round($elapsed.TotalSeconds, 1)
            input     = $payload.usage.input_tokens
            cacheRead = $payload.usage.cache_read_input_tokens
            output    = $payload.usage.output_tokens
            costUsd   = $payload.total_cost_usd
            rubric    = $entry.rubric
            answer    = $payload.result
        }
        $results += [pscustomobject]$record
        $record | ConvertTo-Json -Depth 10 | Out-File (Join-Path $Out "$label.json") -Encoding utf8
        Write-Host ("  {0,-30} {1,7} in  {2,7} cached  {3,6}s" -f $label, $record.input, $record.cacheRead, $record.seconds)
    }
}
finally {
    # Always restore, including on Ctrl-C. A benchmark that leaves the machine's hooks disabled is
    # worse than one that never ran.
    Copy-Item $backup $settings -Force
    Write-Host "settings restored from $backup"
}

$results | Export-Csv (Join-Path $Out 'runs.csv') -NoTypeInformation

# The blind grading sheet, and the key kept apart from it.
#
# Grading is not optional decoration on this measurement — it is half of it. Fewer tokens is not
# better if the answer is wrong, and the failure mode this whole script exists to catch is a warm
# session that answers confidently from a stale memory and stops early, winning on tokens for
# exactly the wrong reason. A sheet that shows the condition beside the answer cannot detect that,
# because nobody grades a column labelled `warm` the same way they grade one labelled `bare`.
#
# So: answers shuffled again, condition and task stripped, opaque ids only. Grade `score` as
# 0 (misses the rubric), 1 (partial), or 2 (meets it), then join to the key on `id`.
$graded = $results | Get-Random -Count $results.Count -SetSeed ($Seed + 1)
$sheet = @()
$key = @()
$index = 0
foreach ($row in $graded) {
    $index++
    $id = 'A{0:D3}' -f $index
    $sheet += [pscustomobject]@{
        id     = $id
        rubric = $row.rubric
        answer = $row.answer
        score  = ''
    }
    $key += [pscustomobject]@{
        id        = $id
        task      = $row.task
        condition = $row.condition
        run       = $row.run
        sessionId = $row.sessionId
    }
}
$sheet | Export-Csv (Join-Path $Out 'grading-sheet.csv') -NoTypeInformation
$key | Export-Csv (Join-Path $Out 'grading-key.csv') -NoTypeInformation

Write-Host ''
Write-Host 'per condition — mean input tokens'
$results | Group-Object condition | ForEach-Object {
    $mean = ($_.Group | Measure-Object input -Average).Average
    Write-Host ("  {0,-6} {1,10:N0}" -f $_.Name, $mean)
}
Write-Host ''
Write-Host 'The brain''s share is (code - warm), not (bare - warm).'
Write-Host ''
Write-Host 'Not finished yet. Fill in the score column of grading-sheet.csv without opening'
Write-Host 'grading-key.csv, then join on id. Report the count of tasks where warm scored WORSE'
Write-Host 'than code with equal prominence — a memory system that misleads with stale context is'
Write-Host 'the failure mode this exists to catch, and the token column cannot see it.'
Write-Host ''
Write-Host "This run's 45 session ids are in sessions.json. Retire what they left in the ledger"
Write-Host 'before running again, or the second number is measuring a brain that has read this'
Write-Host 'benchmark''s own answers.'
