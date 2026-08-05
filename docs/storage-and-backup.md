# Storage and backup — analysis and future levers

Written 2026-08-06. Every number below was measured on the live system, not estimated.
Re-measure before acting on any of it; the shape of the answer will hold, the figures will drift.

---

## Q1 — Where the storage goes, and what it grows to

### Measured, 2026-08-06

| | |
|---|---|
| Brain home | **757 MB** |
| ↳ `projects/` (the ledgers) | 713 MB — 94% |
| ↳ `bin/` (binaries) | 39 MB → **now excluded from backups** |
| ↳ runtime, vault, preferences | ~3 MB |
| Backup root `D:\AgentBrainBackups` | **16.65 GB** across **24** snapshots (~695 MB each) |
| D: drive | 954 GB total, **406 GB free** |

Snapshots are **true copies**. Verified with `fsutil hardlink list` — link count 1. No
deduplication, no compression.

### The projection that matters

The retention policy caps at **66 snapshots** (24 hourly + 30 daily + 12 monthly). Storage does
**not** compound at 15 GB/day. It converges:

| Brain home | Backups at steady state (×66) |
|---|---|
| 750 MB (today) | ~49 GB |
| 2.5 GB (5–6 projects) | ~165 GB |
| 4 GB (heavy use) | ~265 GB |

Against 406 GB free, today is comfortable and 5–6 projects is still comfortable. Pressure starts
past that.

**Brain-home size is the only real variable.** Everything multiplies by 66.

---

## Q2 — Why 24 hourly / 30 daily / 12 monthly

`RetentionPolicy::default()` in `crates/brain-store/src/backup.rs`. This is Grandfather-Father-Son.

A single daily backup gives one recovery point. Break something at 14:00 and notice at 15:00 —
last night's copy already predates nothing useful, and if that one copy is corrupt there is no
second option.

GFS matches granularity to how quickly failures are *discovered*:

| Tier | Catches |
|---|---|
| 24 hourly | Accidental deletion, a bad edit, a crash mid-write — noticed within hours |
| 30 daily | Slow corruption, a bad consolidation writing wrong memories over days |
| 12 monthly | Long-range history: what did the brain know in January? |

Covering a full year at daily granularity would take 365 snapshots (~270 GB). GFS covers the same
span with 66 (~49 GB) — **5× less for the same reach**, by dropping hour-level precision for old
data nobody restores to the exact hour.

**Specific to this system:** the hourly tier is what protects against the brain's *own* bugs. The
2026-08-05 session — repeated service restarts, forced kills, a hung process — is exactly the case
where daily-only backups would offer yesterday's brain as the nearest recovery point.

---

## Q3 — What backups do and do not do

**The brain reads the live ledger, never a backup.** Backups are inert copies, dormant until
something is lost.

| | Backups help? |
|---|---|
| Daily capture, orientation, query, search | **No** — all live-ledger |
| The 5 pain points | **No** — delivered by the live brain |
| "What did I do last month?" | **No** — answered from the ledger, which *is* the long-term memory |
| Disk failure, corruption, accidental deletion | **Yes** |
| Rolling back a bad consolidation | **Yes** — the only undo, since evidence is append-only |

### The nuance that lowers the stakes

Since the rediscovery fix (`crates/brain-service/src/rediscover.rs`), **raw events are
self-healing**. Lose a ledger entirely and the service re-finds every transcript on disk and
re-ingests it automatically.

What is **not** recoverable: consolidated memories, corrections, supersessions, coordination
history, delivery metrics. Those exist nowhere else. And self-healing is bounded by how long
Claude Code and Codex retain their own transcripts before pruning.

**Verdict:** not compulsory for function. Important for the part that took real compute to produce.

---

## Q4 — Levers, ranked by value per unit of effort

### Measured compressibility

```
gzip -6 on the live 600 MB ledger → 237 MB   (2.5×)
```

zstd would reach roughly 3×, considerably faster.

| # | Lever | Saves | Cost | Status |
|---|---|---|---|---|
| 1 | **Stop backing up `bin/`** | ~2.5 GB, grows per deploy | None | ✅ **Done** — commit `e0c36f0` |
| 2 | **Activate segment sealing** | Potentially ~65% of ledger size | Needs wiring — see below | ⬜ Highest remaining value |
| 3 | **Compress snapshots (zstd)** | 49 GB → ~16 GB | CPU on backup; slower restore drills | ⬜ |
| 4 | **Hardlink unchanged files** | Scales *with* project count | Moderate implementation | ⬜ |
| 5 | **Tier by age** | Fast recent restores, small archive | Two code paths | ⬜ |
| 6 | **Reduce retention** | Linear | Directly buys less recovery | ⬜ Last resort |

**Lever 4 deserves emphasis as project count grows.** With 5–6 projects, most are idle in any given
hour, so their ledger files are byte-identical between snapshots and a hardlink shares the storage
instead of copying it. Unlike compression, this improves as projects are added.

**Lever 6 is last deliberately.** Trading recovery granularity for disk space is the worst exchange
on this list. Take it only when the others are exhausted.

### Trigger points

| When | Do |
|---|---|
| Now | ✅ Lever 1 (done) |
| Brain home > 1.5 GB | Lever 3 — compression |
| More than 4 projects | Lever 4 — hardlink dedup |
| Still constrained | Levers 5, then 6 |

### Not a lever: "compact it like Claude Code compacts context"

Context compaction is **lossy summarisation**. A backup that summarised itself would restore a
paraphrase of the database. Backups must be byte-exact. The *tiering* instinct behind the question
is right, and that is lever 5.

---

## Ledger anatomy — why 600 MB for 68k events

Measured on `subscription-agent` (600.1 MB, 68,419 events ≈ 9 KB/event):

| Component | Size | Share |
|---|---|---|
| `raw_json` — original transcript record | 231.6 MB | 39% |
| `payload_json` — normalised form | 159.2 MB | 27% |
| FTS5 index + other indexes | 209.4 MB | 35% |

`payload_json == raw_json` for **0 of 68,419** events, so this is not naive duplication — the two
are a normalised view and a fidelity copy.

By event type:

| Type | Count | Bytes | Average |
|---|---|---|---|
| `agent.responded` | 32,549 | 128.1 MB | 4.0 KB |
| `tool.completed` | 10,425 | 102.8 MB | 10.1 KB |
| `user.prompted` | 1,819 | 60.7 MB | **34.2 KB** |
| `attachment.observed` | 2,323 | 30.7 MB | 13.5 KB |
| `session.compacted` | 12 | 15.5 MB | **1,320 KB** |

Largest single `raw_json`: **3.78 MB**.

### Correction to an earlier hypothesis

An earlier session suggested the ledger was probably bloated by "large tool outputs stored
verbatim," and that trimming would be the highest-leverage change. **The data does not support
that.** The distribution is broad and the content is legitimate. 9 KB/event decomposes as ~2.4 KB
normalised + ~3.5 KB raw + ~3 KB index. There is no obvious waste to trim.

### The actual finding: sealing has never run

```
sealed_segments        rows = 0
event_segment_catalog  rows = 0
events.archived        all 0  (68,419 of 68,419)
```

`crates/brain-store/src/segment.rs` implements a complete cold tier — `SegmentStore::seal()` writes
events into a Zstd-compressed segment, then:

```sql
UPDATE events SET payload_json = '{}', raw_json = '{}', archived = 1
```

A grep for callers finds **only `crates/brain-store/tests/segment_sealing.rs`**. There is no
service loop invoking it and no CLI command exposing it. The machinery is built, tested, and never
wired up.

If activated, it would move the 390 MB of content (65% of the file) into compressed segments,
leaving the hot ledger with metadata and the search index. That is a far larger win than anything
on the backup side — and it shrinks the live brain *and* all 66 snapshots at once.

**Before activating it, confirm:** what `should_seal` uses as its threshold, whether search still
resolves against sealed content (`SegmentCatalog` suggests yes), and how restore-from-segment
performs. Sealing is destructive to the hot rows — it must be provably reversible first.

---

## Housekeeping notes

- `scripts/deploy.ps1` retires locked binaries as `*.old-<stamp>` and clears them on a later run
  via `Remove-RetiredImages`. They linger only while a process holds them —
  `brain-mcp.exe.old-*` persists while Codex is running. Not a leak; it self-clears.
- Backups exclude `bin/` as of `e0c36f0`. A restore reinstates ledgers and configuration; binaries
  come from a deploy, which is already how they reach a working install.
