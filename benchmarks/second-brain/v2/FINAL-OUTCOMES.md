# Secondary Brain benchmark outcomes and competitor context

**Evidence cutoff:** 14 August 2026
**Architecture label:** `current_state`
**Native C0-C4 execution status:** blocked before the first valid sample by insufficient Claude credit

This is the compact outcome surface for the three-goal benchmark. It separates local measurements,
external published measurements, modeled estimates, and unmeasured claims. An external number never
fills a Secondary Brain result cell.

## Current honest verdict

| Goal | C0 native | C4 full Brain | Change | 95% CI | Claude | Codex | Gate | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|---|
| Net native tokens | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | at least 10% saving, adjusted lower bound above 0 | Not run |
| End-to-end coding time | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | at least 10% speedup, adjusted lower bound above 0 | Not run |
| Final-answer pass rate | Not measured | Not measured | Not measured | Not measured | Not measured | Not measured | +5 pp on historical tasks and -2 pp overall safety | Not run |

The implementation and retrieval gates are green. A valid isolated preflight reached the native
Claude provider, which returned `Credit balance is too low` before any turn or token counter existed.
Therefore the Secondary Brain still has no defensible token-saving, coding-speed, or final-answer
quality percentage. Zero-turn attempts are excluded, not scored as zero.

## Native execution evidence available now

| Evidence | Result | What it proves | What it does not prove |
|---|---|---|---|
| Isolated C0-C4 preflight, run `01a00019-2e91-7d53-af0e-0507d7b753e4` | Isolation/profile/condition/native-auth checks passed; provider stopped at insufficient credit | The launcher reaches the real native harness with isolated state and refuses invalid zero-turn samples | Any goal percentage or condition comparison |
| Fresh Claude session `0c11459c-4721-4adc-99c2-6808f0e9e955` | 16 tools discovered; status/search/timeline/evidence succeeded | Claude Brain MCP is installed and useful on demand | Net token, time, or answer-quality lift |
| Fresh Codex session `019fffe2-24f0-74e3-8997-a375c9dce2d9` | 16 tools discovered; status/search/timeline/evidence succeeded | Codex Brain MCP remains operational | Net token, time, or answer-quality lift |
| Lifecycle dashboard | Schema v3; both harnesses visible; 8/8 unattributed MCP requests succeeded | Hook/MCP/capture observability and honest unattributed-session handling | Causal performance improvement |

## Token-savings benchmark context

| System or scenario | Published/measured values | Derived percentage | Evidence class | Can fill Goal 1? |
|---|---:|---:|---|---|
| Secondary Brain C4 versus C0 | Not measured | Not measured | Matched native harness experiment pending | No |
| AgentMemory versus paste-full annual scenario | about 170K versus 19.5M+ tokens/year | about 99.13% at the stated 19.5M floor | Vendor-modeled annual estimate | No |
| AgentMemory versus LLM-summary annual scenario | about 170K versus about 650K tokens/year | about 73.85% | Vendor-modeled annual estimate | No |
| AgentMemory quality fixture | 3,142 retrieved-context versus 22,610 load-all tokens/query | 86.10% | Vendor synthetic context-payload comparison | No |
| AgentMemory scale fixture | 1,924-1,981 top-10 context tokens versus 10,504-2,216,173 load-all tokens | 81.68%-99.91%, published as 82%-100% after rounding | Vendor synthetic scale model | No |

The AgentMemory annual table does not publish matched native Claude/Codex sessions, work-unit counts,
native input/output/cache traces, uncertainty, or a derivation for the yearly totals. Its current CLI
calculates estimated savings as `observations*80 - min(observations,50)*38`; its viewer uses
`observations*80 - sessions*tokenBudget` and a fixed `$0.30/1K` cost assumption. These are useful
payload estimates, not net native model usage. Secondary Brain Goal 1 instead uses the final native
token totals from matched C0-C4 sessions and keeps failed-answer tokens in the denominator.

## Retrieval accuracy

### LongMemEval-S, all 500 questions

| Metric | Secondary Brain, locally reproduced | AgentMemory, published | Difference (Secondary Brain - AgentMemory) |
|---|---:|---:|---:|
| R@5 | 96.0% | 95.2% | +0.8 pp / +0.84% relative |
| R@10 | 98.2% | 98.6% | -0.4 pp / -0.41% relative |
| MRR | 92.2% | 88.2% | +4.0 pp / +4.54% relative |

This is the same public 500-question corpus and the same retrieval metrics, but two independent
harnesses rather than one adapter runner. It is retrieval-only, not the LongMemEval end-to-end QA
score and not proof of better coding answers. Secondary Brain's matched release run processed 23,867
sessions, 246,750 turns, and 243,657 vectors in 13,214.1 seconds. AgentMemory's supplied LongMemEval
report does not publish a comparable latency value.

### coding-agent-life-v1

| Adapter | P@5 | R@5 | Top-5 hit rate | p50 query latency | Evidence status |
|---|---:|---:|---:|---:|---|
| AgentMemory hybrid | 0.240 | 1.000 | 15/15 | 14 ms | Vendor-run public synthetic corpus |
| Grep baseline | 0.227 | 0.967 | 15/15 | 0 ms | Same vendor harness |
| Secondary Brain | Not run | Not run | Not run | Not run | No claim |

P@5 is already at this corpus's mathematical ceiling. The hybrid and grep adapters both hit all 15
queries; one missed gold session in one multi-session temporal query creates the R@5 difference.
The corpus is 15 fictional sessions and 15 queries, so it is a useful regression fixture rather than
strong evidence of general coding-agent superiority.

## Competitor analysis

| Decision dimension | Secondary Brain evidence | AgentMemory evidence | Honest conclusion |
|---|---|---|---|
| Net native tokens | Exact C0-C4 accounting is implemented; provider billing blocked the first valid sample | Annual and dashboard numbers are context models/heuristics | Neither system's supplied evidence proves a causal net native-token saving for this workload |
| Public retrieval | 96.0% R@5, 98.2% R@10, 92.2% MRR locally reproduced | 95.2%, 98.6%, 88.2% published on the same corpus | Secondary Brain leads R@5/MRR and trails R@10; not a controlled adapter head-to-head |
| Small coding-memory fixture | Not run | Hybrid beats grep on one temporal gold session; both hit 15/15 queries | Do not infer broad coding quality from this fixture |
| Retrieval latency | LongMemEval total is measured; no comparable query p50 is published here | 14 ms p50 on coding-agent-life-v1 | Different corpora and timing boundaries; no speed winner |
| Final coding answers | Blind C0-C4 grader and quality gates implemented; native run pending | Supplied benchmarks are retrieval-only or concept-matched synthetic retrieval | Neither source proves superior final coding answers |
| Cross-harness delivery | Three hooks work on Claude and Codex; Brain MCP is installed and read-only-smoked in fresh sessions on both harnesses | MCP/cross-agent support is claimed, not evaluated by these scorecards | Integration is proven operational; performance lift remains unmeasured |
| Auditability | Exact native traces, command log, immutable artifacts, hashes, and confidence intervals | Public corpus/scripts and vendor scorecards; annual-token derivation is absent | Secondary Brain's experiment is stricter, but its three outcome cells remain unmeasured |

## Evidence and provenance

- Secondary Brain LongMemEval-S dataset SHA-256:
  `d6f21ea9d60a0d56f34a05b609c79c88a451d2ae03597821ea3d5a9678c3a442`.
- Secondary Brain matched-hybrid stdout SHA-256:
  `ba79120da5f18586d28a607475a2703edf58f1ee08d9d77f2e78d76dab85cb34`.
- AgentMemory source revision: `2973e4ec4c40d323a08daa34220118010e73a2c3`.
- The six user-supplied Markdown files exactly matched AgentMemory `main` at the evidence cutoff.
  Their downloaded-file SHA-256 values are pinned in `suite.json`.
- Primary external sources: [LongMemEval paper](https://arxiv.org/abs/2410.10813),
  [AgentMemory LongMemEval scorecard](https://github.com/rohitg00/agentmemory/blob/2973e4ec4c40d323a08daa34220118010e73a2c3/benchmark/LONGMEMEVAL.md),
  [coding-agent-life-v1 scorecard](https://github.com/rohitg00/agentmemory/blob/2973e4ec4c40d323a08daa34220118010e73a2c3/docs/benchmarks/2026-05-20-coding-agent-life-v1.md),
  [annual token comparison](https://github.com/rohitg00/agentmemory/blob/2973e4ec4c40d323a08daa34220118010e73a2c3/benchmark/COMPARISON.md),
  [CLI calculator](https://github.com/rohitg00/agentmemory/blob/2973e4ec4c40d323a08daa34220118010e73a2c3/src/cli.ts),
  and [viewer calculator](https://github.com/rohitg00/agentmemory/blob/2973e4ec4c40d323a08daa34220118010e73a2c3/src/viewer/index.html).
- The complete local native evidence for a future run belongs under
  `BRAIN_HOME/runtime/token-benchmarks/<project_id>/<run_id>/`; only a compact non-sensitive
  `BENCHMARK.md` is publishable.

## Final publication rule

Replace `Not measured` only from a completed, valid run of this preregistered suite. Quote absolute
native totals, percentage or percentage-point change, adjusted confidence interval, per-harness
results, invalid-sample percentage, quality safety result, and exact artifact hashes. Keep external
rows labelled `vendor-run`, `vendor-modeled`, or `implementation heuristic`; never relabel them as
Secondary Brain outcomes.
