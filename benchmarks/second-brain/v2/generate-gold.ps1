param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'retrieval-gold')
)

$ErrorActionPreference = 'Stop'
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.Directory]::CreateDirectory($OutputDirectory) | Out-Null

function New-GoldSplit {
    param(
        [Parameter(Mandatory = $true)][string]$Split,
        [Parameter(Mandatory = $true)][string]$Timestamp,
        [Parameter(Mandatory = $true)][int]$UuidBand
    )

    $records = [System.Collections.Generic.List[string]]::new()
    $channels = @('session_start', 'prompt_push', 'historical_pull')
    for ($channelIndex = 0; $channelIndex -lt $channels.Count; $channelIndex++) {
        $channel = $channels[$channelIndex]
        for ($caseIndex = 1; $caseIndex -le 40; $caseIndex++) {
            $expectAbstention = $channel -eq 'prompt_push' -and $caseIndex -gt 20
            $eventBand = $UuidBand + $channelIndex
            $eventId = '{0}0000000-0000-7000-8000-{1:d12}' -f $eventBand, $caseIndex
            $caseId = '{0}-{1}-{2:d3}' -f ($Split -replace '_', '-'), ($channel -replace '_', '-'), $caseIndex
            $fact = '{0} fixture fact {1:d2} for {2}' -f $Split, $caseIndex, $channel
            [object[]]$expectedFacts = @()
            [object[]]$acceptableEvidence = @()
            if (-not $expectAbstention) {
                $expectedFacts = @($fact)
                $acceptableEvidence = @([ordered]@{
                    event_id = $eventId
                    source_locator = 'history/gold-{0}-{1}.jsonl' -f $Split, $channel
                    source_offset = 1000 + ($channelIndex * 10000) + ($caseIndex * 137)
                })
            }
            $record = [ordered]@{
                id = $caseId
                schema_version = 1
                split = $Split
                channel = $channel
                project_alias = 'fixture-project-a'
                query = if ($channel -eq 'session_start') {
                    'Orient a new session with {0}' -f $fact
                } elseif ($channel -eq 'prompt_push') {
                    'Return only newly relevant current-session context for case {0:d2}' -f $caseIndex
                } else {
                    'Find historical evidence for {0}' -f $fact
                }
                as_of = $Timestamp
                expected_facts = $expectedFacts
                acceptable_evidence = $acceptableEvidence
                prohibited_facts = @('cross-project fixture fact', 'evidence newer than as_of')
                expect_abstention = $expectAbstention
            }
            $records.Add(($record | ConvertTo-Json -Compress -Depth 8))
        }
    }
    return $records
}

$calibration = New-GoldSplit -Split 'calibration' -Timestamp '2026-08-01T00:00:00Z' -UuidBand 1
$locked = New-GoldSplit -Split 'locked_test' -Timestamp '2026-08-02T00:00:00Z' -UuidBand 4
[System.IO.File]::WriteAllLines((Join-Path $OutputDirectory 'calibration.jsonl'), $calibration, $utf8NoBom)
[System.IO.File]::WriteAllLines((Join-Path $OutputDirectory 'locked-test.jsonl'), $locked, $utf8NoBom)
