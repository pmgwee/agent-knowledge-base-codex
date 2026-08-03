# Operations, Backup, and Scale Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Keep the secondary brain correct, fast, recoverable, observable, and maintainable as daily agent use grows from thousands to tens of millions of events.

**Architecture:** Recent evidence remains in per-project SQLite WAL stores. Old raw records are sealed into immutable compressed segments with checksummed manifests while searchable summaries and catalog rows stay hot. Backups are copied through SQLite-safe APIs and verified by isolated restore tests.

**Tech Stack:** Rust, SQLite backup API/WAL/FTS5, Zstd, SHA-256, Windows Service Control Manager, structured JSON logs, synthetic deterministic corpus generator.

## Global Constraints

- Complete worktree coordination first.
- Primary release corpus is 12,000 sessions and 6,000,000 events.
- Stress corpus is 120,000 sessions and 60,000,000 events.
- Segment a hot ledger when it reaches 256 MiB or at month end.
- Backups: hourly increment/recent copy, daily verified backup, 30 daily and 12 monthly retained.
- Run an automated isolated restore drill monthly and surface missed/failed drills in health status.
- Recovery targets are RPO at most 1 hour and RTO at most 2 hours on the primary corpus.
- Disk pressure must shed derived work before capture and must never silently delete canonical evidence.
- Every upgrade and restore operates on an isolated copy before replacing active state.

---

### Task 1: Add content-addressed blobs and immutable segment manifests

**Files:**
- Create: crates/brain-store/src/blob.rs
- Create: crates/brain-store/src/segment.rs
- Create: crates/brain-store/src/manifest.rs
- Modify: crates/brain-store/src/lib.rs
- Modify: crates/brain-store/src/migrations.rs
- Create: crates/brain-store/tests/blob_dedup.rs
- Create: crates/brain-store/tests/segment_sealing.rs
- Create: docs/schemas/segment-manifest.md

**Interfaces:**
- Consumes: large event payloads and a sealable ledger range
- Produces: SHA-256 blobs, `.jsonl.zst` segments, immutable manifests, hot catalog rows

- [ ] **Step 1: Write failing deduplication and crash-safety tests**

~~~rust
#[test]
fn identical_large_payloads_share_one_blob_without_cross_project_references() {
    let store = SegmentFixture::two_projects();
    store.append_same_large_payload_to_both();
    assert_eq!(store.physical_blob_count(), 1);
    assert_eq!(store.project_blob_references(project_a()), 1);
    assert_eq!(store.project_blob_references(project_b()), 1);
}

#[test]
fn crash_before_manifest_publish_leaves_hot_rows_queryable() {
    let fixture = SegmentFixture::crash_at(CrashPoint::BeforeManifestRename);
    fixture.seal().unwrap_err();
    assert_eq!(fixture.query_event_count(), fixture.original_event_count());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-store blob_dedup segment_sealing`

Expected: FAIL because blobs and segments are absent.

- [ ] **Step 3: Implement content-addressed blobs**

Hash bytes before compression; write to a temporary same-volume path with create-new semantics; flush; atomically rename to `blobs/sha256/<prefix>/<hash>`. Store MIME, raw/compressed size, and hash. Reference counts are derived and rebuildable.

- [ ] **Step 4: Implement two-phase segment sealing**

Select an immutable event range, stream canonical JSONL ordered by occurrence/source sequence, compress with pinned Zstd settings, calculate hashes/counts/min/max timestamps, validate by reread, then atomically publish the manifest. Only after publication may hot raw payloads be compacted; locator/catalog rows remain.

~~~rust
pub struct SegmentManifest {
    pub format_version: u32,
    pub project_id: ProjectId,
    pub segment_id: uuid::Uuid,
    pub first_event_id: uuid::Uuid,
    pub last_event_id: uuid::Uuid,
    pub event_count: u64,
    pub occurred_min: time::OffsetDateTime,
    pub occurred_max: time::OffsetDateTime,
    pub compressed_sha256: [u8; 32],
    pub uncompressed_sha256: [u8; 32],
}
~~~

- [ ] **Step 5: Run tests**

Run: `cargo test -p brain-store blob_dedup segment_sealing`

Expected: dedup, reference scope, deterministic stream, crash points, corrupt segment, month boundary, and 256 MiB threshold tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-store docs/schemas/segment-manifest.md
git commit -m "feat: seal canonical evidence into verified segments"
~~~

### Task 2: Query hot, warm, and cold evidence through one catalog

**Files:**
- Create: crates/brain-store/src/catalog.rs
- Create: crates/brain-store/src/tiered_query.rs
- Modify: crates/brain-store/src/search.rs
- Create: crates/brain-store/tests/tiered_query.rs
- Create: crates/brain-context/tests/cold_evidence.rs

**Interfaces:**
- Consumes: scoped temporal/text query
- Produces: unified candidates from hot SQLite and sealed segment locators

- [ ] **Step 1: Write failing tier-equivalence tests**

~~~rust
#[test]
fn query_results_are_equivalent_before_and_after_sealing() {
    let fixture = TieredFixture::new();
    let before = fixture.query("OAuth regression", last_year());
    fixture.seal_old_events();
    let after = fixture.query("OAuth regression", last_year());
    assert_eq!(before.logical_ids(), after.logical_ids());
}

#[test]
fn normal_session_start_does_not_decompress_cold_segments() {
    let fixture = TieredFixture::with_cold_history();
    fixture.compile_startup();
    assert_eq!(fixture.segment_open_count(), 0);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-store tiered_query; cargo test -p brain-context cold_evidence`

Expected: FAIL because sealed data is not queryable.

- [ ] **Step 3: Keep searchable catalog data hot**

Retain event identity, project, task/session, type, timestamps, selected search text, evidence links, segment ID, compressed offset/index block, and raw hash. Open cold segments only for explicit evidence expansion or when a top-ranked catalog hit requires raw content.

- [ ] **Step 4: Add an LRU decompression cache with strict bounds**

Cache complete small segments or indexed blocks under configurable byte/item caps. Key by segment hash. Corruption invalidates the cache and returns a visible evidence-unavailable result without inventing content.

- [ ] **Step 5: Run tests**

Run: `cargo test -p brain-store tiered_query; cargo test -p brain-context cold_evidence`

Expected: equivalence, range query, missing/corrupt segment, cache eviction, project scope, and no-normal-start decompression tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-store crates/brain-context
git commit -m "feat: query hot and sealed evidence uniformly"
~~~

### Task 3: Implement verified backup, retention, and isolated restore

**Files:**
- Create: crates/brain-store/src/backup.rs
- Create: crates/brain-store/src/restore.rs
- Create: crates/brain-store/tests/backup_restore.rs
- Create: crates/brain-cli/src/backup.rs
- Create: crates/brain-cli/src/restore.rs
- Modify: crates/brain-cli/src/main.rs
- Create: tests/e2e/disaster_recovery.rs
- Create: docs/operations/backup-and-restore.md

**Interfaces:**
- Consumes: live BRAIN_HOME and retention policy
- Produces: consistent backup set, signed/checksummed inventory, isolated restore and verified cutover

- [ ] **Step 1: Write failing live-backup and corrupt-restore tests**

~~~rust
#[tokio::test]
async fn backup_during_capture_restores_to_a_consistent_cursor_boundary() {
    let fixture = RecoveryFixture::capturing();
    let backup = fixture.backup_while_appending().await;
    let restored = fixture.restore_isolated(backup).await.unwrap();
    assert!(restored.cursors_match_committed_events());
    assert!(restored.integrity_check_passes());
}

#[test]
fn checksum_failure_never_replaces_active_brain() {
    let fixture = RecoveryFixture::with_corrupt_backup();
    assert!(fixture.restore().is_err());
    assert_eq!(fixture.active_brain_hash(), fixture.original_brain_hash());
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-store backup_restore; cargo test --test disaster_recovery`

Expected: FAIL because backup and restore are absent.

- [ ] **Step 3: Implement consistent backup sets**

Use SQLite backup API for each live database; copy immutable segments/blobs by hash; include config, registry, Markdown manifests, migration versions, binary version, and an inventory hash. Write to staging, validate every item, then publish `backup.json` last.

- [ ] **Step 4: Implement retention and safe restore**

Keep hourly restore points sufficient for one-hour RPO, 30 daily, and 12 monthly backups. Restore only to a newly created isolated directory first; run hashes, `PRAGMA integrity_check`, migration compatibility, segment verification, projection verification, and a sample retrieval suite. The service schedules a monthly isolated restore drill, records its inventory hash/duration/result, and removes only the verified drill copy afterward. Cutover requires explicit confirmation and keeps the previous active directory as a rollback copy.

- [ ] **Step 5: Run tests and measure RPO/RTO**

Run:

~~~powershell
cargo test -p brain-store backup_restore
cargo test --test disaster_recovery -- --nocapture
~~~

Expected: live capture, interrupted backup, missing blob, corrupt DB/segment, retention boundary, isolated restore, rollback, RPO, and RTO tests pass on the primary fixture.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-store crates/brain-cli tests/e2e docs/operations/backup-and-restore.md
git commit -m "feat: add verified secondary brain disaster recovery"
~~~

### Task 4: Add health, backpressure, and disk-pressure behavior

**Files:**
- Create: crates/brain-service/src/metrics.rs
- Create: crates/brain-service/src/backpressure.rs
- Modify: crates/brain-service/src/health.rs
- Modify: crates/brain-service/src/capture.rs
- Create: crates/brain-service/tests/backpressure.rs
- Create: crates/brain-service/tests/disk_pressure.rs
- Modify: crates/brain-cli/src/status.rs
- Create: docs/operations/health-and-alerts.md

**Interfaces:**
- Consumes: queue depths, source lag, disk free space, provider health, backup age
- Produces: structured health JSON and deterministic degradation policy

- [ ] **Step 1: Write failing degradation-order tests**

~~~rust
#[tokio::test]
async fn disk_pressure_pauses_optional_derivations_before_capture() {
    let fixture = PressureFixture::at_critical_threshold();
    fixture.run().await;
    assert!(fixture.consolidation_paused());
    assert!(fixture.provider_cache_paused());
    assert!(fixture.capture_continues());
}

#[tokio::test]
async fn full_disk_causes_visible_spool_and_never_advances_cursor() {
    let fixture = PressureFixture::disk_full();
    fixture.capture_once().await;
    assert_eq!(fixture.cursor(), fixture.original_cursor());
    assert!(fixture.health().capture_blocked);
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-service backpressure disk_pressure`

Expected: FAIL because pressure policy is absent.

- [ ] **Step 3: Implement health state and degradation order**

Report capture lag, last cursor/event, quarantine/drift, consolidation backlog, projection lag, provider status, backup age, segment verification, disk bytes/percent, spool size, and p50/p95/p99 hook/service latency. Degrade in order: optional provider refresh, Basic Memory projection, Markdown projection, consolidation, cold cache; protect capture until safe writes are impossible.

- [ ] **Step 4: Add explicit thresholds and recovery hysteresis**

Make warning/critical thresholds configurable with safe defaults. Resume a paused subsystem only after free space/backlog remains above the recovery threshold for two checks. Never auto-delete raw evidence, backups within retention, or unknown records.

- [ ] **Step 5: Run tests**

Run: `cargo test -p brain-service backpressure disk_pressure`

Expected: queue overload, service restart, low disk, full disk, recovery hysteresis, stale backup, and structured-status tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-service crates/brain-cli docs/operations/health-and-alerts.md
git commit -m "feat: degrade safely under backlog and disk pressure"
~~~

### Task 5: Pin formats and implement safe upgrades

**Files:**
- Create: crates/brain-domain/src/version.rs
- Create: crates/brain-store/src/upgrade.rs
- Create: crates/brain-store/tests/upgrade.rs
- Create: crates/brain-cli/src/upgrade.rs
- Modify: crates/brain-cli/src/main.rs
- Create: fixtures/upgrade/v1-ledger.db
- Create: fixtures/upgrade/v1-manifest.json
- Create: docs/operations/upgrades-and-rollback.md

**Interfaces:**
- Consumes: older supported ledger/manifest/config versions
- Produces: staged upgraded copy, compatibility report, verified atomic cutover

- [ ] **Step 1: Write failing forward/backward compatibility tests**

~~~rust
#[test]
fn old_fixture_upgrades_without_changing_raw_event_hashes() {
    let upgraded = upgrade_fixture("v1-ledger.db").unwrap();
    assert_eq!(upgraded.raw_hashes(), fixture_v1_raw_hashes());
}

#[test]
fn newer_unknown_format_is_refused_read_only() {
    let result = open_fixture_with_format_version(999);
    assert!(matches!(result, Err(OpenError::NewerFormat { .. })));
}
~~~

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test -p brain-store upgrade`

Expected: FAIL because explicit format handling is absent.

- [ ] **Step 3: Add version declarations and staged migrations**

Version hook protocol, project registry, ledger, normalized event, memory, Markdown, segment manifest, service API, and backup inventory. `brain upgrade --check` is read-only. `brain upgrade --stage` restores/copies into isolation, migrates, verifies, and reports required downtime.

- [ ] **Step 4: Add rollback behavior**

Cutover retains the previous compatible BRAIN_HOME. If the new service fails its startup self-test, restore the old pointer and emit a diagnostic bundle. Never attempt downgrade migrations in place.

- [ ] **Step 5: Run tests**

Run: `cargo test -p brain-store upgrade; cargo test -p brain-cli upgrade`

Expected: supported upgrade, interrupted migration, raw hash preservation, newer-format refusal, rollback, and projection rebuild tests pass.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-domain crates/brain-store crates/brain-cli fixtures/upgrade docs/operations/upgrades-and-rollback.md
git commit -m "feat: add versioned staged upgrades and rollback"
~~~

### Task 6: Build deterministic primary and stress benchmarks

**Files:**
- Create: tests/scale/generator.rs
- Create: tests/scale/primary.rs
- Create: tests/scale/stress.rs
- Create: tests/scale/memory_quality.rs
- Create: crates/brain-cli/src/benchmark.rs
- Modify: crates/brain-cli/src/main.rs
- Create: docs/operations/scale-benchmarks.md

**Interfaces:**
- Consumes: seed, session/event counts, project/harness distribution
- Produces: reproducible corpus plus JSON latency/throughput/storage/quality report

- [ ] **Step 1: Write failing deterministic-generator test**

~~~rust
#[test]
fn equal_seed_produces_equal_event_and_answer_hashes() {
    let a = generate(42, CorpusSize::Small);
    let b = generate(42, CorpusSize::Small);
    assert_eq!(a.manifest_hash(), b.manifest_hash());
    assert_eq!(a.ground_truth_hash(), b.ground_truth_hash());
}
~~~

- [ ] **Step 2: Run test and confirm failure**

Run: `cargo test --test primary deterministic`

Expected: FAIL because the generator is absent.

- [ ] **Step 3: Generate realistic cross-agent history**

Include Claude/Codex/Hermes sessions, compactions, decisions and reversals, failures and fixes, file changes, deployments, task handoffs, schema unknowns, partial lines, repeated blobs, project-name collisions, worktrees, and known answers for retrieval evaluation.

- [ ] **Step 4: Define production gates**

On 12,000 sessions/6,000,000 events record ingest events/sec, parseable-record capture completeness, explicit gap coverage, replay duplicates, database/segment/blob size, startup p50/p95/p99, text/temporal query p50/p95/p99, historical precision/recall, supersession correctness, project leakage, token reduction against equivalent transcript loading, backup time, restore time, and peak memory. Required: at least 99.9% capture completeness with every gap reported; zero duplicates/leakage; at least 95% evidence-supported historical precision; 100% supersession fixtures correct; at least 80% new-session token reduction; startup p95 at most 500 ms service-side; scoped query p95 at most 1 s; no more than 20% scoped-query latency degradation versus the 1,000-session baseline; hook contract unchanged; RPO/RTO achieved.

- [ ] **Step 5: Add non-blocking stress profile**

The 120,000-session/60,000,000-event run is an overnight/manual release-candidate test. It must complete without unbounded memory growth, integer/cursor overflow, or linear startup scans. Scoped query latency and context size must stay within two times the primary tier. Record results but do not make ordinary `cargo test` generate the corpus.

- [ ] **Step 6: Run primary benchmark**

Run:

~~~powershell
cargo test --test primary --release -- --ignored --nocapture
cargo test --test memory_quality --release -- --ignored --nocapture
~~~

Expected: JSON report satisfies every primary gate. Run stress separately before the final production release.

- [ ] **Step 7: Commit**

~~~powershell
git add crates/brain-cli tests/scale docs/operations/scale-benchmarks.md
git commit -m "test: add production-scale brain benchmarks"
~~~

### Task 7: Package the service and run the operations release gate

**Files:**
- Create: crates/brain-cli/src/service.rs
- Modify: crates/brain-cli/src/main.rs
- Create: tests/e2e/windows_service.rs
- Create: tests/e2e/clean_machine.rs
- Create: docs/operations/install-upgrade-uninstall.md
- Create: docs/operations/runbook.md

**Interfaces:**
- Consumes: release binaries and BRAIN_HOME choice
- Produces: current-user Windows service install/start/stop/status/uninstall and operational runbook

- [ ] **Step 1: Write failing install/uninstall tests**

~~~rust
#[test]
fn uninstall_removes_service_and_hooks_but_preserves_brain_data() {
    let fixture = WindowsInstallFixture::new();
    fixture.install().unwrap();
    fixture.uninstall().unwrap();
    assert!(!fixture.service_exists());
    assert!(!fixture.brain_hooks_exist());
    assert!(fixture.brain_home_exists());
}
~~~

- [ ] **Step 2: Run test and confirm failure**

Run: `cargo test --test windows_service`

Expected: FAIL because packaging commands are absent.

- [ ] **Step 3: Implement service lifecycle commands**

Install exact release binary paths, current-user-access data directories, recovery/restart policy, and structured log rotation. Back up changed harness configs. Uninstall only brain-owned service/hook entries and requires a separate explicit command to archive or remove data.

- [ ] **Step 4: Run clean-machine and operations gates**

Run:

~~~powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --test windows_service -- --ignored --nocapture
cargo test --test clean_machine -- --ignored --nocapture
cargo test --test disaster_recovery -- --nocapture
cargo test --test primary --release -- --ignored --nocapture
~~~

Expected: clean install, restart recovery, upgrade rollback, uninstall data preservation, actual backup restore, and the primary scale gate pass.

- [ ] **Step 5: Complete the runbook**

Cover daily health, backlog, schema drift, provider outage, disk pressure, corrupt segment, failed backup, restore drill, upgrade rollback, worktree/lease incidents, log locations, and diagnostic bundle creation.

- [ ] **Step 6: Commit**

~~~powershell
git add crates/brain-cli tests/e2e docs/operations
git commit -m "feat: package and operate the Windows secondary brain"
~~~

## Operations exit criteria

- Segment sealing and tiered queries preserve logical results and canonical hashes.
- Disk/backlog pressure visibly pauses derived work before capture.
- A real backup restores in isolation with at most one hour of data loss and within two hours.
- Supported upgrades preserve raw hashes and automatically roll back failed startup.
- The 12,000-session/6,000,000-event corpus meets latency, correctness, and resource gates.
- The 120,000-session/60,000,000-event stress run demonstrates bounded startup and memory behavior before production release.
- Clean-machine install, service restart, and uninstall preserve user data and unrelated agent configuration.
