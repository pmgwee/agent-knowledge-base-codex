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
| Query degradation | 4.0% (then gated at 20%; now diagnostic only) |
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

The final release hook latency gate also passed with warm p95 9.46 ms and p99
9.79 ms against a 50 ms p95 limit.

## Ten-times stress gate

**Status: attempted, terminated before reporting. Not yet qualified.**

The deterministic stress profile (120,000 sessions and 60,000,000 events) is
implemented as an ignored/manual release-candidate test. The primary run
temporarily required about 22.58 GB for the source, verified backup, and
isolated restore, so the 10-times profile needs roughly 226 GB of scratch space
before safety margin. The system volume had only 88.1 GB free, but drive D: had
about 347 GB, which is sufficient.

An attempt ran on 2026-08-03 with temporary storage redirected to D::

```powershell
$env:TEMP = 'D:\AgentBrainStress'; $env:TMP = $env:TEMP
cargo test --release -p brain-cli --test stress `
  stress_corpus_keeps_startup_query_and_memory_bounds -- --ignored --nocapture
```

It ingested for 8.9 hours, reaching about 74.95 GiB — roughly 92% of the
expected corpus — with process memory flat near 24 MiB throughout, which is the
bounded-memory behaviour the gate exists to demonstrate. It then stopped at
13:32:57 without reaching the backup phase and without emitting a report.

A second attempt on 2026-08-03 17:49 behaved identically, stopping at 02:21:12
after 8.5 hours and 68.52 GiB.

**Both were killed by a system-initiated reboot, not by any fault in the
system under test.** Its stdout and stderr contain no panic, assertion or
backtrace; they simply stop. The System event log records
`Microsoft-Windows-Kernel-Power` event 109, "the kernel power manager has
initiated a shutdown transition", nine seconds after the first run's last write
and ten seconds after the second's. The machine booted five times in the three
days spanning both attempts.

An eleven-hour gate cannot complete on a host that restarts roughly twice a
day. Suspend automatic restarts for the duration of the run, or run the gate on
a host that does not restart unattended. This is an environment constraint, not
a defect, and it is why the run leaves a durable log: without one, two
consecutive failures looked identical to an unexplained crash.

Operationally this matters beyond the benchmark. A host that restarts twice a
day will terminate the brain service abruptly at the same rate, so crash
recovery is a routine path rather than an exceptional one. Each unclean stop
leaves an uncheckpointed write-ahead log for the next process to recover, which
was measured at 193 seconds for 1.5 GB.

Two defects that attempt exposed are now fixed in `3998d9c`:

- the report was written inside the temporary corpus directory, so any normal
  completion would have deleted it along with the corpus;
- the run was launched without redirecting stdout to a file, so its console
  output had no durable destination once the launching session ended.

Re-run with output redirected to a log so an interrupted attempt remains
diagnosable, and check progress sparsely rather than in a polling loop:

```powershell
$env:TEMP = 'D:\AgentBrainStress'; $env:TMP = $env:TEMP
cargo test --release -p brain-cli --test stress `
  stress_corpus_keeps_startup_query_and_memory_bounds -- --ignored --nocapture `
  *> D:\AgentBrainStress\stress-run.log
```

Not running a disk-destructive workload on an undersized system volume remains
an explicit safety constraint, not a waived correctness gate.

### Near-ten-times measurement from the interrupted corpus

The interrupted attempt left a usable 73.5 GB ledger holding **58,218,000
events**, about 97% of the stress target and 9.7 times the primary corpus. It
was measured read-only on 2026-08-03 with the diagnostic described in
`benchmark.rs` (`existing_corpus_startup_and_cold_retrieval`), run twice to
separate one-time costs from steady state:

| Measurement | 6,000,000 events | 58,218,000 events | Gate |
| --- | ---: | ---: | ---: |
| Startup p95 | 3.265 ms | 1.936 ms warm, 2.463 ms after reboot | 500 ms |
| Cold query p95 | 4.92 ms | 23.28 ms warm, 162.53 ms after reboot | 1 second |
| Full count scan | not measured | 3.96 s warm, ~80 s after reboot | not gated |

Startup is flat across a tenfold increase in history, which is the property the
architecture was designed around, and both gates pass with wide margin.

Cold retrieval is not flat. It grew roughly 33 times for a 9.7-times increase in
events, so it degrades faster than linearly while still sitting well inside the
one-second ceiling. Interactive use is unaffected because the bounded
scoped-query cache serves repeated retrieval in microseconds; the cold figure is
what a first query after a restart costs. If the trend holds, the cold ceiling
would be approached somewhere beyond a few hundred million events, which is the
point at which segment sealing and partition routing would need to carry more of
the load.

Two one-time costs are worth knowing operationally. An uncheckpointed
write-ahead log left by an unclean shutdown is recovered by whichever process
opens the ledger next, and a 1.5 GB log took 193 seconds. An empty operating
system page cache multiplied cold retrieval by seven and a full scan by twenty.
Neither is on the startup path, and neither affects the gates above.

This measurement covers the performance half of the stress profile only. Capture
completeness, replay deduplication, project leakage, precision and recall, token
reduction, and backup and restore at ten times scale still require a complete
run, because they depend on ground truth the interrupted corpus never finished
writing.
