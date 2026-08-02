use anyhow::{Result, bail, ensure};
use brain_domain::{NormalizedEvent, ProjectId, SourceCursor, WorktreeId};

use crate::{NormalizeContext, ReadOutcome, SourceAdapter, SourceDescriptor};

pub struct AdapterConformanceSubject<'a> {
    pub adapter: &'a dyn SourceAdapter,
    pub source: SourceDescriptor,
    pub context: NormalizeContext,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterConformanceReport {
    pub adapter_name: &'static str,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub raw_records: usize,
    pub normalized_events: usize,
    pub committed_cursor: SourceCursor,
}

/// Exercises invariants every native source adapter must preserve. Format-specific
/// suites still own mutation cases such as JSONL partial lines or SQLite WAL rows.
pub fn assert_adapter_conformance(
    subject: AdapterConformanceSubject<'_>,
) -> Result<AdapterConformanceReport> {
    let first_fingerprint = subject.adapter.fingerprint(&subject.source)?;
    let second_fingerprint = subject.adapter.fingerprint(&subject.source)?;
    ensure!(
        first_fingerprint == second_fingerprint,
        "{} fingerprint is not stable",
        subject.adapter.name()
    );

    let first = require_batch(
        subject
            .adapter
            .read_increment(&subject.source, &SourceCursor::start())?,
        subject.adapter.name(),
    )?;
    ensure!(
        !first.records.is_empty(),
        "{} conformance fixture has no records",
        subject.adapter.name()
    );
    ensure!(
        first.next_cursor.byte_offset == first.last_complete_newline,
        "{} cursor passed its proven record boundary",
        subject.adapter.name()
    );
    ensure!(
        first.next_cursor.file_identity.is_some(),
        "{} committed cursor has no stable source identity",
        subject.adapter.name()
    );
    let mut prior_end = 0;
    for record in &first.records {
        ensure!(
            record.byte_offset >= prior_end,
            "{} record offsets are not monotonic",
            subject.adapter.name()
        );
        ensure!(
            record.next_byte_offset > record.byte_offset,
            "{} record did not advance its proven boundary",
            subject.adapter.name()
        );
        ensure!(
            record.next_byte_offset <= first.next_cursor.byte_offset,
            "{} record exceeds committed cursor",
            subject.adapter.name()
        );
        ensure!(
            record.raw_hash.iter().any(|byte| *byte != 0),
            "{} record has an empty raw hash",
            subject.adapter.name()
        );
        prior_end = record.next_byte_offset;
    }

    let first_events = normalize_all(&subject, &first.records)?;
    let repeated_events = normalize_all(&subject, &first.records)?;
    ensure!(
        !first_events.is_empty(),
        "{} fixture normalized to no evidence",
        subject.adapter.name()
    );
    ensure!(
        first_events.len() == repeated_events.len(),
        "{} normalization is not repeatable",
        subject.adapter.name()
    );
    for (first_event, repeated_event) in first_events.iter().zip(&repeated_events) {
        ensure_semantically_equal(first_event, repeated_event, subject.adapter.name())?;
        ensure!(
            first_event.project_id == subject.context.project_id,
            "{} leaked or replaced the registered project scope",
            subject.adapter.name()
        );
        ensure!(
            first_event.worktree_id == subject.context.worktree_id,
            "{} leaked or replaced the registered worktree scope",
            subject.adapter.name()
        );
        let source_record = first
            .records
            .iter()
            .find(|record| {
                i64::try_from(record.byte_offset).ok() == Some(first_event.source_offset)
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{} emitted evidence without a source record",
                    subject.adapter.name()
                )
            })?;
        ensure!(
            first_event.raw_hash == source_record.raw_hash,
            "{} did not retain the native raw hash",
            subject.adapter.name()
        );
    }

    let replay = require_batch(
        subject
            .adapter
            .read_increment(&subject.source, &SourceCursor::start())?,
        subject.adapter.name(),
    )?;
    let replay_events = normalize_all(&subject, &replay.records)?;
    ensure!(
        replay_events.len() == first_events.len(),
        "{} replay changed the event count",
        subject.adapter.name()
    );
    for (first_event, replay_event) in first_events.iter().zip(&replay_events) {
        ensure_semantically_equal(first_event, replay_event, subject.adapter.name())?;
    }

    match subject
        .adapter
        .read_increment(&subject.source, &first.next_cursor)?
    {
        ReadOutcome::NoChange => {}
        other => bail!(
            "{} replay from committed cursor was not empty: {other:?}",
            subject.adapter.name()
        ),
    }

    Ok(AdapterConformanceReport {
        adapter_name: subject.adapter.name(),
        project_id: subject.context.project_id,
        worktree_id: subject.context.worktree_id,
        raw_records: first.records.len(),
        normalized_events: first_events.len(),
        committed_cursor: first.next_cursor,
    })
}

fn require_batch(outcome: ReadOutcome, adapter_name: &str) -> Result<crate::RawRecordBatch> {
    match outcome {
        ReadOutcome::Batch(batch) => Ok(batch),
        other => bail!("{adapter_name} conformance fixture did not return a batch: {other:?}"),
    }
}

fn normalize_all(
    subject: &AdapterConformanceSubject<'_>,
    records: &[crate::RawRecord],
) -> Result<Vec<NormalizedEvent>> {
    records
        .iter()
        .map(|record| subject.adapter.normalize(record, &subject.context))
        .collect::<Result<Vec<_>>>()
        .map(|groups| groups.into_iter().flatten().collect())
}

fn ensure_semantically_equal(
    left: &NormalizedEvent,
    right: &NormalizedEvent,
    adapter_name: &str,
) -> Result<()> {
    ensure!(
        left.project_id == right.project_id,
        "{adapter_name} changed project ID"
    );
    ensure!(
        left.worktree_id == right.worktree_id,
        "{adapter_name} changed worktree ID"
    );
    ensure!(
        left.task_id == right.task_id,
        "{adapter_name} changed task ID"
    );
    ensure!(
        left.harness == right.harness,
        "{adapter_name} changed harness"
    );
    ensure!(
        left.native_session_id == right.native_session_id,
        "{adapter_name} changed native session ID"
    );
    ensure!(
        left.native_turn_id == right.native_turn_id,
        "{adapter_name} changed native turn ID"
    );
    ensure!(
        left.event_type == right.event_type,
        "{adapter_name} changed event type"
    );
    ensure!(
        left.occurred_at == right.occurred_at,
        "{adapter_name} changed occurrence time"
    );
    ensure!(
        left.source_locator == right.source_locator,
        "{adapter_name} changed source locator"
    );
    ensure!(
        left.source_offset == right.source_offset,
        "{adapter_name} changed source offset"
    );
    ensure!(
        left.source_schema == right.source_schema,
        "{adapter_name} changed source schema"
    );
    ensure!(
        left.raw_hash == right.raw_hash,
        "{adapter_name} changed raw hash"
    );
    ensure!(
        left.idempotency_key == right.idempotency_key,
        "{adapter_name} changed idempotency key"
    );
    ensure!(
        left.git_head == right.git_head,
        "{adapter_name} changed Git head"
    );
    ensure!(
        left.git_branch == right.git_branch,
        "{adapter_name} changed Git branch"
    );
    ensure!(
        left.payload == right.payload,
        "{adapter_name} changed payload"
    );
    ensure!(left.raw == right.raw, "{adapter_name} changed raw evidence");
    Ok(())
}
