# Claude walking-skeleton operator verification

This runbook verifies the local Windows foundation: stable project registration, incremental Claude JSONL capture, durable SQLite evidence, bounded startup context, fail-open hooks, restart-safe replay, and surgical hook removal. This phase does not require an LLM, embedding API, Basic Memory, CodeGraph, LLM Wiki, or Obsidian.

## 1. Build the release binaries

Run from the repository root in PowerShell:

```powershell
$env:BRAIN_HOME = "$env:USERPROFILE\AgentBrain"
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release --workspace
$brainBin = (Resolve-Path ".\target\release\brain.exe").Path
$hookBin = (Resolve-Path ".\target\release\brain-hook.exe").Path
$serviceBin = (Resolve-Path ".\target\release\brain-service.exe").Path
```

## 2. Register one project

```powershell
$projectRoot = (Resolve-Path "C:\path\to\project").Path
$registration = (& $brainBin register $projectRoot) | ConvertFrom-Json
$registration | ConvertTo-Json -Depth 5
```

Registration searches `%USERPROFILE%\.claude\projects` for JSONL files whose record-level `cwd` is inside the canonical project root. It does not infer identity from Claude's encoded directory name. To disable discovery, add `--no-discover-claude`; to use a different source root, add `--claude-projects-root C:\path\to\claude\projects`.

Expected fields include `project_id`, `worktree_id`, `project_root`, `ledger_path`, `service_config_path`, and `claude_sources`. Repeating the command must preserve the same IDs and ledger path.

## 3. Inspect configured health

```powershell
& $brainBin status --project $registration.project_id --json
```

Expected JSON fields include:

- `persisted_events` and `last_event_at`
- `source_count` and `backlog_bytes`
- `quarantined_records`
- `unresolved_capture_gaps`
- `healthy`

At this foundation stage, `status` is a durable ledger/configuration snapshot. It does not claim that the background service process is currently running.

## 4. Start the capture and hook service

```powershell
$brainService = Start-Process -FilePath $serviceBin -WindowStyle Hidden -PassThru
Start-Sleep -Milliseconds 500
```

The service reconciles every two seconds in addition to filesystem notifications. It starts the named-pipe hook endpoint before ongoing capture work.

## 5. Install the Claude user hook

```powershell
$claudeSettings = "$env:USERPROFILE\.claude\settings.json"
& $brainBin install-hooks claude --settings $claudeSettings --hook-executable $hookBin
```

The result reports `changed`, `settings_path`, and `backup_path`. When settings already exist, `backup_path` points to a timestamped sibling file. The installer validates JSON, preserves unrelated settings/hooks, adds one `SessionStart` command, and atomically replaces the settings file. Re-running the command returns `changed: false` and creates no duplicate.

Start a new Claude Code session in the registered project. The hook should return an evidence-linked orientation under 1,500 tokens. If the service is unavailable, `brain-hook.exe` exits successfully with `{}` and atomically spools the envelope under `%BRAIN_HOME%\spool`; Claude startup is not blocked.

## 6. Query the same bounded orientation manually

```powershell
& $brainBin query --project $registration.project_id "current project orientation"
```

Every selected memory block must include an `Evidence: event:<uuid>` citation. Historical text is labeled untrusted and must be checked against current code and test results.

## 7. Simulate a service restart

```powershell
Stop-Process -Id $brainService.Id
$brainService = Start-Process -FilePath $serviceBin -WindowStyle Hidden -PassThru
Start-Sleep -Seconds 3
& $brainBin status --project $registration.project_id --json
```

`persisted_events` must not increase from replayed duplicates. New complete JSONL records should be captured after restart from the committed byte cursor.

## 8. Remove only the brain-owned Claude hook

```powershell
& $brainBin uninstall-hooks claude --settings $claudeSettings --hook-executable $hookBin
Stop-Process -Id $brainService.Id
```

Uninstall creates a timestamped backup, removes only the command matching this `brain-hook.exe` plus its Claude arguments, and preserves every unrelated setting and hook. It does not restore an old whole-file backup over newer user changes.

## Release gates

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" fmt --all --check
& "$env:USERPROFILE\.cargo\bin\cargo.exe" clippy --workspace --all-targets -- -D warnings
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test --workspace
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test --test claude_walking_skeleton -- --nocapture
& "$env:USERPROFILE\.cargo\bin\cargo.exe" test --release -p brain-hook --test latency -- --ignored --nocapture
```

The release contract is cold-process p95 at or below 100 ms and warm p95 at or below 50 ms on this Windows machine.
