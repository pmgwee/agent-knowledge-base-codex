use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use brain_context::{ContextCompiler, ContextQuery, token_count};
use brain_domain::{
    Authority, EventBatch, EventType, Harness, MemoryKind, MemoryRecord, MemoryScope, MemoryStatus,
    NormalizedEvent, ProjectId, SourceCursor, WorktreeId,
};
use brain_store::{EventLedger, SearchQuery, SearchSource};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const ORIENTATION_EVENT_LIMIT: usize = 64;
const HISTORICAL_LIMIT: usize = 20;
const MIN_PROMPT_CHARACTERS: usize = 24;
const MIN_SHARED_TERMS: usize = 2;
const MAX_PUSHED_MEMORIES: usize = 4;
const MID_SESSION_TOKENS: usize = 400;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSplit {
    Calibration,
    LockedTest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalChannel {
    SessionStart,
    PromptPush,
    HistoricalPull,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AcceptableEvidence {
    pub event_id: uuid::Uuid,
    pub source_locator: String,
    pub source_offset: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RetrievalGoldCase {
    pub id: String,
    pub schema_version: u32,
    pub split: RetrievalSplit,
    pub channel: RetrievalChannel,
    pub project_alias: String,
    pub query: String,
    #[serde(with = "time::serde::rfc3339")]
    pub as_of: time::OffsetDateTime,
    pub expected_facts: Vec<String>,
    pub acceptable_evidence: Vec<AcceptableEvidence>,
    pub prohibited_facts: Vec<String>,
    pub expect_abstention: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PercentageMetric {
    pub numerator: u64,
    pub denominator: u64,
    pub percent: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RetrievalMetrics {
    pub precision: PercentageMetric,
    pub recall: PercentageMetric,
    pub f1: PercentageMetric,
    pub mrr: PercentageMetric,
    pub ndcg_at_5: PercentageMetric,
    pub fact_accuracy: PercentageMetric,
    pub citation_precision: PercentageMetric,
    pub citation_coverage: PercentageMetric,
    pub faithfulness: PercentageMetric,
    pub freshness: PercentageMetric,
    pub abstention: PercentageMetric,
    pub harmful_push: PercentageMetric,
    pub healthy_silence: PercentageMetric,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RetrievalCaseResult {
    pub id: String,
    pub split: RetrievalSplit,
    pub channel: RetrievalChannel,
    pub valid: bool,
    pub invalid_reason: Option<String>,
    pub returned_citations: Vec<String>,
    pub relevant_citations: Vec<String>,
    pub unresolved_citations: Vec<String>,
    pub output_text: String,
    pub abstained: bool,
    pub fact_hits: u64,
    pub expected_fact_count: u64,
    pub prohibited_fact_hits: u64,
    pub reciprocal_rank: f64,
    pub ndcg_at_5: f64,
    pub faithful: bool,
    pub fresh: bool,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct RetrievalBenchmarkReport {
    pub schema_version: u32,
    pub project_id: ProjectId,
    pub executable_sha256: String,
    pub config_sha256: String,
    pub case_count: u64,
    pub valid_cases: u64,
    pub invalid_cases: u64,
    /// True only for the reproducible one-case-per-ledger gold fixture, never for a production
    /// project ledger. This prevents fixture scores from masquerading as live-corpus evidence.
    #[serde(default)]
    pub fixture_generated: bool,
    pub metrics: RetrievalMetrics,
    pub cases: Vec<RetrievalCaseResult>,
}

#[derive(Clone, Debug)]
pub struct RetrievalBenchmarkOptions {
    pub gold_path: PathBuf,
    pub output_dir: PathBuf,
    pub split: RetrievalSplit,
    pub executable_sha256: String,
    pub config_sha256: String,
}

#[derive(Default)]
struct Counts {
    relevant: u64,
    returned: u64,
    acceptable: u64,
    reciprocal_rank_sum: f64,
    ndcg_sum: f64,
    ranked_cases: u64,
    fact_hits: u64,
    expected_facts: u64,
    citation_covered_facts: u64,
    faithful: u64,
    fresh: u64,
    valid_cases: u64,
    abstention_correct: u64,
    abstention_cases: u64,
    harmful_pushes: u64,
    negative_pushes: u64,
    healthy_silences: u64,
}

struct RetrievalOutput {
    text: String,
    citations: Vec<String>,
}

struct ResolvedCitation {
    supports: HashSet<uuid::Uuid>,
    fresh: bool,
}

pub fn evaluate_retrieval_cases(
    ledger: &EventLedger,
    project_id: ProjectId,
    cases: &[RetrievalGoldCase],
    executable_sha256: impl Into<String>,
    config_sha256: impl Into<String>,
) -> Result<RetrievalBenchmarkReport> {
    let mut results = Vec::with_capacity(cases.len());
    let mut counts = Counts::default();
    for case in cases {
        let result = evaluate_case(ledger, project_id, case)?;
        if result.valid {
            accumulate(&mut counts, case, &result);
        }
        results.push(result);
    }
    let valid_cases = results.iter().filter(|result| result.valid).count() as u64;
    let invalid_cases = results.len() as u64 - valid_cases;
    Ok(RetrievalBenchmarkReport {
        schema_version: 1,
        project_id,
        executable_sha256: executable_sha256.into(),
        config_sha256: config_sha256.into(),
        case_count: results.len() as u64,
        valid_cases,
        invalid_cases,
        fixture_generated: false,
        metrics: metrics(&counts),
        cases: results,
    })
}

/// Evaluate the preregistered gold records against reproducible isolated ledgers.
///
/// Each case receives only its declared evidence. That is intentional: SessionStart is a bounded
/// orientation, not a query-specific ranker, so putting 40 mutually exclusive expected answers in
/// one ledger would make every case share the same output and manufacture low precision. The
/// fixture tests component behavior, while `run_retrieval_benchmark` remains the live-ledger path.
pub fn evaluate_retrieval_fixture_cases(
    project_id: ProjectId,
    cases: &[RetrievalGoldCase],
    executable_sha256: impl Into<String>,
    config_sha256: impl Into<String>,
) -> Result<RetrievalBenchmarkReport> {
    let mut results = Vec::with_capacity(cases.len());
    let mut counts = Counts::default();
    for case in cases {
        let mut ledger = EventLedger::open_in_memory(project_id)?;
        materialize_fixture_case(&mut ledger, project_id, case)?;
        let result = evaluate_case(&ledger, project_id, case)?;
        if result.valid {
            accumulate(&mut counts, case, &result);
        }
        results.push(result);
    }
    let valid_cases = results.iter().filter(|result| result.valid).count() as u64;
    let invalid_cases = results.len() as u64 - valid_cases;
    Ok(RetrievalBenchmarkReport {
        schema_version: 1,
        project_id,
        executable_sha256: executable_sha256.into(),
        config_sha256: config_sha256.into(),
        case_count: results.len() as u64,
        valid_cases,
        invalid_cases,
        fixture_generated: true,
        metrics: metrics(&counts),
        cases: results,
    })
}

pub fn run_retrieval_fixture_benchmark(
    project_id: ProjectId,
    options: &RetrievalBenchmarkOptions,
) -> Result<RetrievalBenchmarkReport> {
    let cases = read_gold_cases(&options.gold_path, options.split)?;
    let report = evaluate_retrieval_fixture_cases(
        project_id,
        &cases,
        &options.executable_sha256,
        &options.config_sha256,
    )?;
    write_report(&options.output_dir, &report)?;
    Ok(report)
}

fn materialize_fixture_case(
    ledger: &mut EventLedger,
    project_id: ProjectId,
    case: &RetrievalGoldCase,
) -> Result<()> {
    if case.expect_abstention {
        ensure!(
            case.acceptable_evidence.is_empty(),
            "abstention fixture cannot declare positive evidence"
        );
        return Ok(());
    }
    ensure!(
        !case.expected_facts.is_empty(),
        "positive fixture case has no expected fact"
    );
    let occurred_at = case.as_of - time::Duration::days(1);
    let content = case.expected_facts.join("; ");
    for evidence in &case.acceptable_evidence {
        let digest = Sha256::digest(format!("{}:{}", case.id, evidence.event_id).as_bytes());
        let mut raw_hash = [0_u8; 32];
        raw_hash.copy_from_slice(&digest);
        ledger.append_batch(&EventBatch {
            source_id: format!("retrieval-fixture-{}", case.id),
            events: vec![NormalizedEvent {
                event_id: evidence.event_id,
                project_id,
                worktree_id: WorktreeId(uuid::Uuid::nil()),
                task_id: None,
                harness: Harness::Codex,
                native_session_id: format!("fixture-{}", case.id),
                native_turn_id: None,
                event_type: EventType::AgentResponded,
                occurred_at,
                observed_at: occurred_at,
                source_locator: evidence.source_locator.clone(),
                source_offset: evidence.source_offset,
                source_schema: "retrieval-gold-fixture:v1".to_owned(),
                raw_hash,
                idempotency_key: raw_hash,
                git_head: None,
                git_branch: None,
                payload: serde_json::json!({ "content": content }),
                raw: serde_json::json!({ "content": content }),
            }],
            quarantined: Vec::new(),
            capture_gaps: Vec::new(),
            next_cursor: SourceCursor::start(),
        })?;
        if case.channel == RetrievalChannel::PromptPush {
            let memory_id = uuid::Uuid::from_u128(
                evidence.event_id.as_u128() ^ 0x8000_0000_0000_0000_0000_0000_0000_0001,
            );
            let version_id = uuid::Uuid::from_u128(
                evidence.event_id.as_u128() ^ 0x4000_0000_0000_0000_0000_0000_0000_0002,
            );
            ledger.append_memory(&MemoryRecord {
                id: memory_id,
                version_id,
                scope: MemoryScope::Project(project_id),
                worktree_id: None,
                task_id: None,
                kind: MemoryKind::Fact,
                title: format!("Newly relevant current session context for {}", case.id),
                content: format!("{content}. Current session context for {}.", case.id),
                valid_from: occurred_at,
                valid_to: None,
                recorded_at: occurred_at,
                confidence: 1.0,
                authority: Authority::DerivedMemory,
                evidence_ids: vec![evidence.event_id],
                supersedes: Vec::new(),
                status: MemoryStatus::Current,
            })?;
        }
    }
    Ok(())
}

pub fn run_retrieval_benchmark(
    ledger: &EventLedger,
    project_id: ProjectId,
    options: &RetrievalBenchmarkOptions,
) -> Result<RetrievalBenchmarkReport> {
    let cases = read_gold_cases(&options.gold_path, options.split)?;
    let report = evaluate_retrieval_cases(
        ledger,
        project_id,
        &cases,
        &options.executable_sha256,
        &options.config_sha256,
    )?;
    write_report(&options.output_dir, &report)?;
    Ok(report)
}

pub fn read_gold_cases(path: &Path, expected: RetrievalSplit) -> Result<Vec<RetrievalGoldCase>> {
    let file = File::open(path).with_context(|| format!("open gold set {}", path.display()))?;
    let mut cases = Vec::new();
    let mut ids = HashSet::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let case: RetrievalGoldCase = serde_json::from_str(&line)
            .with_context(|| format!("parse gold record at line {}", index + 1))?;
        ensure!(case.schema_version == 1, "unsupported gold schema");
        ensure!(case.split == expected, "gold split does not match command");
        ensure!(
            ids.insert(case.id.clone()),
            "duplicate gold case {}",
            case.id
        );
        ensure!(
            case.expect_abstention || !case.acceptable_evidence.is_empty(),
            "positive gold case {} has no acceptable evidence",
            case.id
        );
        cases.push(case);
    }
    ensure!(!cases.is_empty(), "gold set is empty");
    Ok(cases)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

fn evaluate_case(
    ledger: &EventLedger,
    project_id: ProjectId,
    case: &RetrievalGoldCase,
) -> Result<RetrievalCaseResult> {
    if let Some(reason) = validate_gold_evidence(ledger, case)? {
        return Ok(invalid_case(case, reason));
    }
    let started = std::time::Instant::now();
    let output = retrieve(ledger, project_id, case)?;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let acceptable = case
        .acceptable_evidence
        .iter()
        .map(|evidence| evidence.event_id)
        .collect::<HashSet<_>>();
    let mut relevant = Vec::new();
    let mut unresolved = Vec::new();
    let mut relevance = Vec::with_capacity(output.citations.len());
    let mut fresh = true;
    for citation in &output.citations {
        match resolve_citation(ledger, citation, case.as_of)? {
            Some(resolved) => {
                fresh &= resolved.fresh;
                let supports = !resolved.supports.is_disjoint(&acceptable);
                relevance.push(supports);
                if supports {
                    relevant.push(citation.clone());
                }
            }
            None => {
                fresh = false;
                relevance.push(false);
                unresolved.push(citation.clone());
            }
        }
    }
    let normalized_output = normalize(&output.text);
    let fact_hits = case
        .expected_facts
        .iter()
        .filter(|fact| normalized_output.contains(&normalize(fact)))
        .count() as u64;
    let prohibited_fact_hits = case
        .prohibited_facts
        .iter()
        .filter(|fact| normalized_output.contains(&normalize(fact)))
        .count() as u64;
    let reciprocal_rank = relevance
        .iter()
        .position(|relevant| *relevant)
        .map_or(0.0, |rank| 1.0 / (rank as f64 + 1.0));
    let ndcg_at_5 = ndcg(&relevance, acceptable.len());
    let abstained = output.text.trim().is_empty() && output.citations.is_empty();
    Ok(RetrievalCaseResult {
        id: case.id.clone(),
        split: case.split,
        channel: case.channel,
        valid: true,
        invalid_reason: None,
        returned_citations: output.citations,
        relevant_citations: relevant,
        unresolved_citations: unresolved,
        output_text: output.text,
        abstained,
        fact_hits,
        expected_fact_count: case.expected_facts.len() as u64,
        prohibited_fact_hits,
        reciprocal_rank,
        ndcg_at_5,
        faithful: prohibited_fact_hits == 0 && relevance.iter().all(|value| *value),
        fresh,
        elapsed_ms,
    })
}

fn retrieve(
    ledger: &EventLedger,
    project_id: ProjectId,
    case: &RetrievalGoldCase,
) -> Result<RetrievalOutput> {
    match case.channel {
        RetrievalChannel::SessionStart => session_start(ledger, project_id, case.as_of),
        RetrievalChannel::PromptPush => prompt_push(ledger, project_id, case),
        RetrievalChannel::HistoricalPull => historical_pull(ledger, project_id, case),
    }
}

fn session_start(
    ledger: &EventLedger,
    project_id: ProjectId,
    as_of: time::OffsetDateTime,
) -> Result<RetrievalOutput> {
    let mut query = ContextQuery::for_worktree(project_id, WorktreeId(uuid::Uuid::nil()));
    query.as_of = Some(as_of);
    let compiled =
        ContextCompiler::from_ledger_as_of(ledger, project_id, ORIENTATION_EVENT_LIMIT, as_of)?
            .compile(query)?;
    Ok(RetrievalOutput {
        text: compiled.text,
        citations: compiled
            .citations
            .into_iter()
            .map(|citation| citation.key)
            .collect(),
    })
}

fn historical_pull(
    ledger: &EventLedger,
    project_id: ProjectId,
    case: &RetrievalGoldCase,
) -> Result<RetrievalOutput> {
    let hits = ledger.search(
        &SearchQuery::text(project_id, &case.query)
            .as_of(case.as_of)
            .without_session_diversity()
            .without_access_recording()
            .with_limit(HISTORICAL_LIMIT),
    )?;
    Ok(output_from_hits(hits))
}

fn prompt_push(
    ledger: &EventLedger,
    project_id: ProjectId,
    case: &RetrievalGoldCase,
) -> Result<RetrievalOutput> {
    if case.query.trim().chars().count() < MIN_PROMPT_CHARACTERS {
        return Ok(RetrievalOutput {
            text: String::new(),
            citations: Vec::new(),
        });
    }
    let hits = ledger.search(
        &SearchQuery::text(project_id, &case.query)
            .as_of(case.as_of)
            .memories_only()
            .without_access_recording()
            .with_limit(8),
    )?;
    let mut text = Vec::new();
    let mut citations = Vec::new();
    let mut spent = 0;
    for hit in hits {
        if citations.len() >= MAX_PUSHED_MEMORIES
            || shared_terms(&case.query, &hit.title, &hit.text) < MIN_SHARED_TERMS
        {
            continue;
        }
        let line = format!("- {} — {}", hit.title, first_sentence(&hit.text));
        let cost = token_count(&line);
        if spent + cost > MID_SESSION_TOKENS {
            continue;
        }
        spent += cost;
        text.push(line);
        citations.push(format!("memory:{}", hit.source_id));
    }
    Ok(RetrievalOutput {
        text: text.join("\n"),
        citations,
    })
}

fn output_from_hits(hits: Vec<brain_store::SearchHit>) -> RetrievalOutput {
    let mut text = Vec::new();
    let mut citations = Vec::new();
    for hit in hits {
        text.push(format!("{}: {}", hit.title, hit.text));
        let prefix = match hit.source {
            SearchSource::Event => "event",
            SearchSource::Memory => "memory",
        };
        citations.push(format!("{prefix}:{}", hit.source_id));
    }
    RetrievalOutput {
        text: text.join("\n"),
        citations,
    }
}

fn validate_gold_evidence(
    ledger: &EventLedger,
    case: &RetrievalGoldCase,
) -> Result<Option<String>> {
    for expected in &case.acceptable_evidence {
        let Some(event) = ledger.event(expected.event_id)? else {
            return Ok(Some(format!(
                "expected event {} is unreadable or outside the project",
                expected.event_id
            )));
        };
        if event.source_locator != expected.source_locator {
            return Ok(Some(format!(
                "source locator mismatch for {}",
                expected.event_id
            )));
        }
        if event.source_offset != expected.source_offset {
            return Ok(Some(format!(
                "source offset mismatch for {}",
                expected.event_id
            )));
        }
        if event.occurred_at > case.as_of {
            return Ok(Some(format!(
                "expected event {} is newer than cutoff",
                expected.event_id
            )));
        }
    }
    Ok(None)
}

fn resolve_citation(
    ledger: &EventLedger,
    citation: &str,
    as_of: time::OffsetDateTime,
) -> Result<Option<ResolvedCitation>> {
    let Some((kind, id)) = citation.split_once(':') else {
        return Ok(None);
    };
    let Ok(id) = uuid::Uuid::parse_str(id) else {
        return Ok(None);
    };
    match kind {
        "event" => Ok(ledger.event(id)?.map(|event| ResolvedCitation {
            supports: HashSet::from([event.event_id]),
            fresh: event.occurred_at <= as_of,
        })),
        "memory" => {
            let Some(memory) = ledger.memory_version(id)? else {
                return Ok(None);
            };
            if !memory.status.is_readable()
                || memory.valid_from > as_of
                || memory.valid_to.is_some_and(|until| as_of >= until)
            {
                return Ok(None);
            }
            let mut fresh = true;
            for evidence_id in &memory.evidence_ids {
                let Some(event) = ledger.event(*evidence_id)? else {
                    return Ok(None);
                };
                fresh &= event.occurred_at <= as_of;
            }
            Ok(Some(ResolvedCitation {
                supports: memory.evidence_ids.into_iter().collect(),
                fresh,
            }))
        }
        _ => Ok(None),
    }
}

fn invalid_case(case: &RetrievalGoldCase, reason: String) -> RetrievalCaseResult {
    RetrievalCaseResult {
        id: case.id.clone(),
        split: case.split,
        channel: case.channel,
        valid: false,
        invalid_reason: Some(reason),
        returned_citations: Vec::new(),
        relevant_citations: Vec::new(),
        unresolved_citations: Vec::new(),
        output_text: String::new(),
        abstained: false,
        fact_hits: 0,
        expected_fact_count: case.expected_facts.len() as u64,
        prohibited_fact_hits: 0,
        reciprocal_rank: 0.0,
        ndcg_at_5: 0.0,
        faithful: false,
        fresh: false,
        elapsed_ms: 0,
    }
}

fn accumulate(counts: &mut Counts, case: &RetrievalGoldCase, result: &RetrievalCaseResult) {
    counts.valid_cases += 1;
    counts.returned += result.returned_citations.len() as u64;
    counts.relevant += result.relevant_citations.len() as u64;
    counts.acceptable += case.acceptable_evidence.len() as u64;
    if !case.acceptable_evidence.is_empty() {
        counts.reciprocal_rank_sum += result.reciprocal_rank;
        counts.ndcg_sum += result.ndcg_at_5;
        counts.ranked_cases += 1;
    }
    counts.fact_hits += result.fact_hits;
    counts.expected_facts += result.expected_fact_count;
    if !result.relevant_citations.is_empty() {
        counts.citation_covered_facts += result.fact_hits;
    }
    counts.faithful += u64::from(result.faithful);
    counts.fresh += u64::from(result.fresh);
    if case.expect_abstention {
        counts.abstention_cases += 1;
        counts.abstention_correct += u64::from(result.abstained);
    }
    if case.channel == RetrievalChannel::PromptPush && case.expect_abstention {
        counts.negative_pushes += 1;
        counts.harmful_pushes += u64::from(!result.abstained);
        counts.healthy_silences += u64::from(result.abstained);
    }
}

fn metrics(counts: &Counts) -> RetrievalMetrics {
    let precision = metric(counts.relevant, counts.returned);
    let recall = metric(counts.relevant.min(counts.acceptable), counts.acceptable);
    let f1_percent = match (precision.percent, recall.percent) {
        (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
        (Some(_), Some(_)) => Some(0.0),
        _ => None,
    };
    RetrievalMetrics {
        precision: precision.clone(),
        recall,
        f1: PercentageMetric {
            numerator: counts.relevant,
            denominator: counts.returned.saturating_add(counts.acceptable),
            percent: f1_percent,
        },
        mrr: float_metric(counts.reciprocal_rank_sum, counts.ranked_cases),
        ndcg_at_5: float_metric(counts.ndcg_sum, counts.ranked_cases),
        fact_accuracy: metric(counts.fact_hits, counts.expected_facts),
        citation_precision: precision,
        citation_coverage: metric(counts.citation_covered_facts, counts.expected_facts),
        faithfulness: metric(counts.faithful, counts.valid_cases),
        freshness: metric(counts.fresh, counts.valid_cases),
        abstention: metric(counts.abstention_correct, counts.abstention_cases),
        harmful_push: metric(counts.harmful_pushes, counts.negative_pushes),
        healthy_silence: metric(counts.healthy_silences, counts.negative_pushes),
    }
}

fn metric(numerator: u64, denominator: u64) -> PercentageMetric {
    PercentageMetric {
        numerator,
        denominator,
        percent: (denominator != 0).then(|| numerator as f64 * 100.0 / denominator as f64),
    }
}

fn float_metric(sum: f64, denominator: u64) -> PercentageMetric {
    PercentageMetric {
        numerator: sum.round() as u64,
        denominator,
        percent: (denominator != 0).then(|| sum * 100.0 / denominator as f64),
    }
}

fn ndcg(relevance: &[bool], acceptable: usize) -> f64 {
    if acceptable == 0 {
        return 0.0;
    }
    let dcg = relevance
        .iter()
        .take(5)
        .enumerate()
        .filter(|(_, relevant)| **relevant)
        .map(|(rank, _)| 1.0 / ((rank + 2) as f64).log2())
        .sum::<f64>();
    let ideal = (0..acceptable.min(5))
        .map(|rank| 1.0 / ((rank + 2) as f64).log2())
        .sum::<f64>();
    if ideal == 0.0 { 0.0 } else { dcg / ideal }
}

fn shared_terms(prompt: &str, title: &str, body: &str) -> usize {
    fn terms(text: &str) -> HashSet<String> {
        text.split(|character: char| !character.is_alphanumeric())
            .filter(|word| word.chars().count() >= 4)
            .map(str::to_lowercase)
            .collect()
    }
    let asked = terms(prompt);
    let held = terms(&format!("{title} {body}"));
    asked.intersection(&held).count()
}

fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    let head = trimmed.split_once(". ").map_or(trimmed, |(head, _)| head);
    if head.chars().count() <= 160 {
        head.to_owned()
    } else {
        format!("{}…", head.chars().take(159).collect::<String>())
    }
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn write_report(output_dir: &Path, report: &RetrievalBenchmarkReport) -> Result<()> {
    if output_dir.exists() {
        bail!(
            "retrieval benchmark output already exists: {}",
            output_dir.display()
        );
    }
    fs::create_dir_all(output_dir)?;
    let cases_path = output_dir.join("retrieval-cases.jsonl");
    let mut cases = BufWriter::new(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(cases_path)?,
    );
    for result in &report.cases {
        serde_json::to_writer(&mut cases, result)?;
        cases.write_all(b"\n")?;
    }
    cases.flush()?;
    let summary_path = output_dir.join("summary.json");
    let mut summary = BufWriter::new(
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(summary_path)?,
    );
    serde_json::to_writer_pretty(&mut summary, report)?;
    summary.write_all(b"\n")?;
    summary.flush()?;
    Ok(())
}
