# Codex lifecycle-hook integration

Codex continuation uses the official additive lifecycle-hook system in `%USERPROFILE%\.codex\hooks.json`. It does not replace the legacy top-level `notify` command in `config.toml`; on this machine that command is already owned by Codex Computer Use.

Official behavior used by this integration:

- Codex merges matching hooks from multiple active hook files.
- `SessionStart` runs for `startup`, `resume`, `clear`, and `compact`.
- The hook receives JSON on stdin with `session_id`, `transcript_path`, `cwd`, `hook_event_name`, and `source`.
- `hookSpecificOutput.additionalContext` becomes developer context.
- Non-managed hooks must be reviewed and trusted with `/hooks`.

Reference: [OpenAI Codex hooks documentation](https://learn.chatgpt.com/docs/hooks).

## Build and install

```powershell
$env:BRAIN_HOME = "$env:USERPROFILE\AgentBrain"
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release --workspace
$brainBin = (Resolve-Path ".\target\release\brain.exe").Path
$hookBin = (Resolve-Path ".\target\release\brain-hook.exe").Path
$codexHooks = "$env:USERPROFILE\.codex\hooks.json"

& $brainBin install-hooks codex --settings $codexHooks --hook-executable $hookBin
```

The installer validates JSON, creates a timestamped sibling backup when the file exists, preserves unrelated hook groups, adds one bounded `SessionStart` handler, and atomically replaces `hooks.json`. Re-running it returns `changed: false` without creating a duplicate. It never edits `%USERPROFILE%\.codex\config.toml` or its `notify` value.

The installed handler uses:

- matcher `^(startup|resume|clear|compact)$`
- one-second Codex process timeout, while `brain-hook.exe` keeps its own 250 ms fail-open deadline
- `additionalContextLimit: 1500`
- the Codex-native JSON response envelope

## Trust and verify

Start Codex and run `/hooks`. Review the exact absolute `brain-hook.exe --harness codex` command and trust it. Codex skips a new or changed non-managed hook until this review is complete.

Hooks are enabled by default. If `[features].hooks = false` exists in an active Codex config layer, remove that override or set it to `true`; the brain installer does not change unrelated Codex configuration.

With `brain-service.exe` running, start or resume a Codex session inside the registered project. The returned orientation must:

- contain only evidence from that registered project;
- cite each selected block as `Evidence: event:<uuid>`;
- remain at or below 1,500 tokens;
- work after automatic/manual compaction through `source: compact`;
- return `{}` without blocking Codex if the service is unavailable.

The same project ledger and compiler serve Claude Code and Codex; there is no provider-specific memory copy.

## Remove only the brain-owned hook

```powershell
& $brainBin uninstall-hooks codex --settings $codexHooks --hook-executable $hookBin
```

Uninstall creates another timestamped backup and surgically removes only the command matching this hook executable and `--harness codex`. It preserves all other Codex hooks and does not overwrite newer changes with an old whole-file backup.
