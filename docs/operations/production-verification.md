# Production verification

Verified on 2026-08-03 from commit `e10cd59` on Windows 11
10.0.26200 (64-bit), AMD Ryzen 7 5800H (8 cores/16 logical processors),
27.9 GiB visible memory, Rust/Cargo 1.88.0.

## Primary production gate

Command:

```powershell
cargo test --release -p brain-cli --test primary `
  primary_production_corpus_passes_all_gates -- --ignored --nocapture
```

The deterministic seed-42 corpus passed every gate with no failures:

| Measurement | Result |
| --- | ---: |
| Sessions | 12,000 |
| Generated/captured events | 6,000,000 / 6,000,000 |
| Capture completeness | 100% |
| Explicit gap coverage | yes |
| Replay duplicate rows | 0 |
| Ingest throughput | 2,030.68 events/s |
| Canonical storage | 8,082,862,080 bytes |
| Peak bounded batch | 1,300,920 bytes |
| Startup p50 / p95 / p99 | 2.2786 / 3.2650 / 3.2650 ms |
| Warm scoped query p50 / p95 / p99 | 0.0070 / 0.0078 / 0.0081 ms |
| 1,000-session query p95 baseline | 0.0075 ms |
| Query degradation | 4.0% (maximum 20%) |
| Historical precision / recall | 100% / 100% |
| Supersession fixtures | 100% correct |
| Cross-project leakage | 0 hits |
| New-session token reduction | 99.99875% |
| Backup / isolated restore | 286.55 / 272.42 seconds |
| RPO / RTO | pass / pass |
| Hook contract | unchanged |

The query measurement is the warm service-side path used by the long-running
brain service. The canonical query cache is capped at 128 entries and 16 MiB,
cleared after local writes, and invalidated after commits from other
connections through SQLite `data_version`. The preceding uncached six-million
event diagnostic measured raw FTS p95 at 4.92 ms, also well under the one-second
absolute gate.

The release hook latency gate also passed with warm p95 12.27 ms and p99 14.38
ms against a 50 ms p95 limit.

## Ten-times stress gate

The deterministic stress profile (120,000 sessions and 60,000,000 events) is
implemented as an ignored/manual release-candidate test. It was not executed on
this system drive. The primary run temporarily required about 22.58 GB for the
source, verified backup, and isolated restore; the 10-times profile therefore
requires approximately 226 GB of scratch space before safety margin. Only 88.1
GB was free at final verification.

Run the stress gate only after directing temporary and benchmark storage to a
scratch volume with at least 250 GB free:

```powershell
cargo test --release -p brain-cli --test stress `
  stress_corpus_keeps_startup_query_and_memory_bounds -- --ignored --nocapture
```

Not running a disk-destructive workload on an undersized system volume is an
explicit safety constraint, not a waived correctness gate. The same generator,
assertions, bounded batches, startup/query limits, and recovery checks are
present for execution on suitable hardware.
