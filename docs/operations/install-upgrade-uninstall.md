# Windows install, upgrade, and uninstall

AgentBrain runs as three least-privilege, current-user Windows Scheduled Tasks:

- `AgentBrain.Service` starts at user logon, ignores duplicate starts, has no
  execution time limit, and restarts after failure.
- `AgentBrain.Backup` creates and verifies an hourly restore point, then applies
  the 24-hour/30-day/12-month retention policy.
- `AgentBrain.RestoreDrill` restores the latest verified point in isolation on
  the first day of each month and records the result.

This packaging intentionally does not require an administrator or LocalSystem
access. The user must be logged in for the capture service to run. Structured
JSON logs rotate daily under `BRAIN_HOME\runtime\logs`, with 14 files retained.

## Clean install

Build all release binaries, register at least one project, then install using
absolute paths. Keep backups and restore drills outside `BRAIN_HOME`.

```powershell
cargo build --workspace --release
target\release\brain.exe --brain-home D:\AgentBrain register D:\projects\project-a
target\release\brain.exe --brain-home D:\AgentBrain service install `
  --service-executable target\release\brain-service.exe `
  --brain-executable target\release\brain.exe `
  --backup-root E:\AgentBrainBackups `
  --drill-root E:\AgentBrainDrills `
  --install-hooks `
  --hook-executable target\release\brain-hook.exe
target\release\brain.exe --brain-home D:\AgentBrain service status
```

The installer validates the service configuration and all executable paths,
backs up hook configuration through the existing hook installer, writes a
versioned install manifest, and starts the capture task. It refuses to replace
same-named tasks when no matching AgentBrain manifest exists.

## Upgrade

Do not replace a running binary. First create a backup and stage the upgrade:

```powershell
brain --brain-home D:\AgentBrain backup maintain --root E:\AgentBrainBackups
brain --brain-home D:\AgentBrain upgrade check
brain --brain-home D:\AgentBrain upgrade stage --destination E:\AgentBrainUpgradeStage
brain --brain-home D:\AgentBrain service stop
```

Build or copy the new release to a versioned directory, rerun `service install`
with the new exact executable paths, then run `service status`, a project
query, and a hook smoke test. If any gate fails, stop the task and rerun the
installer with the prior release paths. The staged brain and verified backup
remain the rollback sources.

## Uninstall

```powershell
brain --brain-home D:\AgentBrain service uninstall
```

Uninstall ends and removes only the three manifest-owned tasks and the exact
AgentBrain hook entries recorded at install. It removes its task XML and
install manifest. It deliberately preserves canonical brain data, Obsidian
projection, logs, external LLM Wiki vault, CodeGraph indexes, backups, and
restore-drill reports. Data removal or archival is a separate explicit manual
operation after a verified backup.
