# Backup recovery — implementation plan

Written 2026-08-16, from a full audit of the live system (D: census, snapshot inventories,
Task Scheduler state, and the backup/upgrade/benchmark code paths). Every figure below was
measured on 2026-08-16 unless noted. This plan supersedes the lever table in
`docs/storage-and-backup.md` (written 2026-08-06 against a 757 MB brain home).

Status legend: ⬜ not started · 🔶 in progress · ✅ done

---

## What happened

The backup root `D:\AgentBrainBackups` reached **346.2 GB** with D: at **~70 GB free**, and the
hourly backup task was dying at its 2-hour execution limit (`Last Result -1073741510`), so
backups were failing as often as succeeding — each success writing **+24.3 GB**.

| Measured 2026-08-16 | Value |
|---|---|
| Backup root | **346.2 GB** (`brain.exe dashboard` backup_root_bytes; Explorer shows ~342 GiB) |
| ↳ 31 published snapshots | 236.2 GB — 0.68 GB each on Aug 5 → 3.49 GB Aug 14 → **24.29 GB each** from Aug 14 20:54 |
| ↳ 10 orphaned `.staging-*` dirs | **~110 GB** — partial copies from killed runs, 5 created Aug 16; never cleaned |
| Newest snapshot contents | 24.29 GB / 192,214 files — `runtime/token-benchmarks` **22.05 GB / 164,033 files (91%)**, ledgers 1.97 GB / 3 files, vault 0.09 GB / 27,713 files, models 0.17 GB / 6 files |
| Live brain home (C:) | 26.3 GB, of which the actual brain (`projects/` + `vault/` + `models/`) is ~2.3 GB |
| Retention | Working as designed — GFS 24/30/12 retains all 31 current points; `prune --apply` deletes nothing today |

**Root cause:** `REBUILDABLE_DIRECTORIES = ["bin"]` (`crates/brain-store/src/backup.rs`) is the
only backup exclusion. The token-benchmark workflow stores run artifacts — frozen-brain ledger
copies, full git checkouts, per-attempt harness homes — under
`~/AgentBrain/runtime/token-benchmarks/` **by design** (`ensure_runtime_isolation` requires run
roots beneath BRAIN_HOME), and the backup copies all of it, every hour, with per-file fsync and
double hashing. Two individually-sane designs composing into a 24 GB snapshot. The storage doc's
"brain home > 1.5 GB → act" trigger was passed 16× silently because nothing alarms.

**Dashboard's 56.6 GB was not a bug in the number** — it was a true reading from 2026-08-13,
served frozen: the UI runs in remote mode (Upstash creds in `.env.local`) and the
`AgentBrain.PushSnapshot` task is Disabled; both its spawn timeouts (45 s push, 90 s API route)
are now shorter than the ~150 s `brain dashboard` walk, so it would freeze again even if
re-enabled.

Safety facts established during the audit (why the steps below are safe):

- `.sqlite` snapshots go through the SQLite online backup API (WAL-safe, verified against
  page-level evidence); a killed run never publishes a partial snapshot — it orphans a staging
  dir instead.
- Deleting snapshot dirs by hand is safe: retention is stateless (recomputed from `read_dir` +
  each dir's own `backup.json`), drill reports live under a separate root, and
  `latest_verified_backup` rescans disk — but keep ≥1 verified snapshot or the drill task fails.
- This machine has `LongPathsEnabled=0`: PowerShell `Remove-Item` and size walks silently fail
  on >260-char paths. Delete with `cmd /c rd /s /q "\\?\…"`, measure with `brain.exe` or
  robocopy. (Two of the audit's own early numbers were 2–3× undercounts for exactly this reason.)

---

## Phase A — stop the bleeding (manual, no code)

### Step 0 — emergency reclaim (~330 GB) 🔶

1. `schtasks /Change /TN "AgentBrain.Backup" /DISABLE` — don't race an in-flight run.
2. Delete the 10 `.staging-*` dirs (~110 GB) and all but the newest **2** published snapshots
   (~226 GB), each via `cmd /c rd /s /q "\\?\D:\AgentBrainBackups\<dir>"`.
3. Verify and re-enable the task (re-enable happens after Step 1 ships, so the first
   post-cleanup run is already a clean one).

**Accept:** D: free ~70 → ~400 GB; `brain.exe dashboard` backup_root_bytes ≈ 25–50 GB;
remaining snapshots all carry `backup.json`.

### Step 0b — optional stopgap (only if Step 1 cannot ship the same day)

Manually prune graded runs' `frozen-brain/` + `checkout/` + `attempts/` on C: (same `\\?\`
deletion). The next hourly snapshot drops to ~2.3 GB with zero code changes. ⚠️ Ends those
runs' resumability and post-hoc hash verification — only for runs already graded + reported.

## Phase B — fix the machine (code; each commit auto-deploys via `.githooks/post-commit`)

### Step 1 — stop copying rebuildable scaffolding 🔶

`REBUILDABLE_DIRECTORIES += "runtime/token-benchmarks"` in `crates/brain-store/src/backup.rs`.
Three verified implementation constraints:

- The entry **must** be the multi-segment relative path `"runtime/token-benchmarks"` — entries
  are joined onto the brain home and prefix-matched, so a bare `"token-benchmarks"` compiles,
  runs, and excludes nothing.
- `UpgradeManager::check()` walks the brain home with its own exclusion-free file collection
  and aggregates raw-event hashes from **every** SQLite — including benchmark frozen brains —
  then requires hash equality against the backup-restored copy. The exclusion must be applied
  to that walk too, or `brain upgrade stage` fails whenever a benchmark run exists.
- Extend the pinned test `backups_skip_rebuildable_binaries_but_keep_everything_else`
  (`crates/brain-store/tests/backup_restore.rs`) so the exclusion is contract, not accident.

`models/` (174 MB, manually downloaded) is deliberately **kept** in snapshots for now —
excluding it silently reverts a restored brain to keyword-only search until someone re-downloads
the checkpoints. Revisit at Step 6; if excluded then, document the re-download step in the
restore runbook.

**Accept:** next maintain publishes in minutes; snapshot inventory contains no
`runtime/token-benchmarks` entries; snapshot ≈ 2.3 GB / ~28k files (from 24.3 GB / 192k).
**Effect:** kills the 2-hour-timeout failure mode at its root — a 2.3 GB copy finishes in
minutes, so no more killed runs and no more staging orphans.

### Step 2 — self-healing: staging sweeper + prune policy flags 🔶

- Age-based `.staging-*` sweep in `backup maintain`/`prune` — the markdown projection already
  has the identical pattern for its own staging dirs (`crates/brain-store/src/markdown.rs`,
  abandoned-staging collection by age). Threshold 6 h: comfortably beyond the task's PT2H
  execution limit, so a live run's staging is never swept. Reported in the retention report as
  a distinct `swept_staging` list; dry-run respects `dry_run`.
- `backup prune` gains `--hourly/--daily/--monthly` overrides (both `prune` and `maintain`
  currently hardcode `RetentionPolicy::default()`, so the CLI cannot express a one-time
  reclaim today — that's why Step 0 had to be manual).

**Accept:** kill a backup mid-run → next maintain sweeps the orphan; `prune --hourly 2 --apply`
deletes down to 2 hourly points.

### Step 3 — first production restore drill ⬜

Run `backup drill-latest` once. The `AgentBrain.RestoreDrill` task has **never fired**
(Last Run 30/11/1999) — the restore path is completely untested in production. At 2.3 GB this
is minutes. **This gates Phase C:** no compression, no sealing, before one successful drill.

**Accept:** report written under `AgentBrainDrills/reports/`; all integrity checks green.

### Step 4 — benchmark prune on C: (stop regrowth at the source) ⬜

`brain benchmark prune`, gated on *graded + report built*: keep manifest/preflight/samples/
grades/report/configs/raw (~2.3 MB per run), delete `frozen-brain/` + `checkout/` + `attempts/`
(85–99% of the current 5.62 GB across 7 runs), write a `pruned.json` marker, and refuse
`run --execute` on pruned runs with a real message instead of a raw IO error. Nothing ever
cleans this tree today — `brain benchmark retire` writes a marker and deletes nothing by design.

**Accept:** `runtime/token-benchmarks/` ≤ ~20 MB per retained run; dashboard brain_home_bytes
drops ~26.3 → ~2.5 GB (frees C: too — 53.6 GB free there).

### Step 5 — make the dashboard honest and alarming ⬜

Re-enable `AgentBrain.PushSnapshot`; replace the ~150 s full-tree walk in the push path with a
sum of per-snapshot inventory `total_bytes` (cheap, immune to the timeout that froze it twice);
surface `generated_at`/`pushed_at` staleness on the tiles so a frozen number cannot impersonate
a live one; add a free-space / brain-home-size alarm (the doc's own 1.5 GB trigger was passed
16× silently). Mirror any new snapshot fields in `lib/snapshot-types.ts` (dashboard repo) and
refresh the stale figures in `docs/storage-and-backup.md`.

**Accept:** UI number tracks disk within one push cycle; a stale snapshot shows a visible badge.

## Phase C — get structurally small (lever stack, in dependency order)

The doc's levers, re-ranked against 2026-08-16 numbers. Operates on the clean ~2.3 GB snapshot
that Step 1 produces — never run these against 24 GB snapshots full of scaffolding.

### Step 6 — Lever #3 first: zstd-compress snapshots ⬜

~2.7× measured on ledger content → steady state **~150 GB → ~55 GB**. Lossless; no retrieval
risk. Prerequisite: Step 3's drill. `verify_tree` moves to hashing compressed bytes; drills get
slower — measure and document. Decide the `models/` question here.

### Step 7 — Lever #2: activate segment sealing ⬜

Still the highest structural value: the only lever that shrinks the **live brain on C:** as well
as every snapshot, and the only answer to ledger growth at the source (ledgers went 0.68 →
2.13 GB in 8 days). Sealed segments are immutable once written, which is what makes Step 8
deterministic. The backup side is already ready (`classify()` tags `.jsonl.zst` as `Segment`).
Prerequisites per `docs/storage-and-backup.md`: confirm `should_seal`'s threshold, prove search
resolves sealed content, benchmark segment restore — then a LongMemEval regression run before
it goes live.

### Step 8 — Lever #4: hardlink unchanged files, *after* sealing ⬜

Today's `.sqlite` snapshot copies come from the SQLite backup API, whose output is **not
guaranteed byte-identical between runs** — dedup on the ledgers must be measured, not assumed.
Post-sealing, segment files are immutable and dedupe becomes deterministic; vault's 27k static
files dedupe reliably today. Same volume required (D: ✓); restore must copy, never move.

### Explicitly rejected

- **Lever #5 tier by age** — two code paths to save ~25 GB that Steps 6–7 already remove.
- **Lever #6 retention trim** — GFS retains all 31 current points, so trimming trades real
  recovery for arithmetic that doesn't move. The multiplier was never the problem.
- **Relocating benchmark artifacts outside the brain home** — fights the
  `ensure_runtime_isolation` security invariant across four call sites.

---

## Trajectory

| Checkpoint | Backup root | Per snapshot |
|---|---|---|
| 2026-08-16 (audit) | 346 GB, +24.3 GB per successful run | 24.3 GB / 192k files |
| After Phase A | ~25–50 GB | 24.3 GB (unless 0b done) |
| After Steps 1–2 | ~5 GB | **2.3 GB / ~28k files** |
| Steady state (66 × 2.3 GB) | ~150 GB | — |
| + Step 6 (zstd) | **~55 GB** | ~0.85 GB |
| + Step 7 (sealing) | **~28 GB** | — |
| + Step 8 (hardlinks) | **~15–20 GB** | — |

## Hardening backlog (found during the audit, deliberately not bundled)

- `backup.rs` opens the source ledger with `Connection::open` (read-write, CREATE flag) — a
  source that vanished mid-run would fabricate an empty database and publish it as a valid
  empty snapshot. Open read-only.
- `collect_files` hard-fails the **entire backup** on any symlink under the brain home — one
  junction in a future benchmark harness-home is a total backup outage.
- Non-SQLite kinds are copied without atomicity; a file appended mid-copy is torn and the hash
  check "locks in" the torn state. Bounded risk today (segments are write-once), worth a
  stability pass alongside Step 6.
- `directory_bytes` + `unwrap_or(0)` renders a walk error as "0 bytes used" — make partial
  failure visible when the storage panel is reworked (Step 5).
