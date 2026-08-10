use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use brain_context::{
    ContextCompiler, ContextQuery, LiveState, ProviderConfig, ProviderResult, token_count,
};
use brain_coordination::{
    CoordinationStore, LeaseError, Overlap, RENEWAL_INTERVAL, SessionIdentity, overlap,
};
use brain_domain::{Harness, HookEnvelope, HookReply, ProjectId, WorktreeId};
use brain_store::{EventLedger, ProviderCacheStore};
use sha2::Digest;

/// How many recent events an orientation is compiled from.
///
/// Named because two places must agree on it: the compile itself, and [`ProjectHookHandler::warm`],
/// which exists to pull exactly these pages into cache before a session start needs them. Warming a
/// different number would warm the wrong pages and still look like it worked.
const ORIENTATION_EVENT_LIMIT: usize = 500;

#[derive(Clone, Debug)]
pub struct HookProjectBinding {
    pub project_root: PathBuf,
    pub project_id: ProjectId,
    pub worktree_id: WorktreeId,
    pub ledger_path: PathBuf,
    pub global_preferences_path: Option<PathBuf>,
    /// Where the embedding model lives, so a mid-session push can use the vector channel.
    ///
    /// Carried explicitly rather than derived from `ledger_path`. The derivation that already
    /// existed for the wiki provider walks two parents from the ledger's directory and lands on the
    /// project id, not the brain home — a latent bug that has never surfaced only because that
    /// provider is disabled.
    pub brain_home: PathBuf,
}

/// What the handler produced: a reply to send, and — when an orientation was compiled — a metric
/// to record *after* that reply has been delivered.
pub struct HookOutcome {
    pub reply: HookReply,
    /// `Some` only when an orientation was compiled. Recording it before the client has the reply
    /// is what made `context_deliveries` count compilations instead of receipts.
    pub delivery: Option<PendingDelivery>,
}

impl HookOutcome {
    /// A reply with nothing to record — no project matched, or the event was not a session start.
    pub fn bare(reply: HookReply) -> Self {
        Self {
            reply,
            delivery: None,
        }
    }
}

/// Stage a delivery for a hook that pushed text without compiling an orientation.
///
/// `UserPromptSubmit` and `SessionEnd` return lease warnings and mid-session pushes, and for a long
/// while they recorded nothing at all — so a per-hook panel read zero for them whether they were
/// working or dead, which is the one thing such a panel exists to tell apart. They have no
/// citations and no memory/coordination split to report, so those are zero and honestly so: the
/// count and the token total are the whole truth about this kind of push.
fn staged_push(
    binding: &HookProjectBinding,
    envelope: &HookEnvelope,
    text: &str,
) -> Option<PendingDelivery> {
    let total_tokens = token_count(text) as u64;
    if total_tokens == 0 {
        return None;
    }
    Some(PendingDelivery {
        ledger_path: binding.ledger_path.clone(),
        project_id: binding.project_id,
        delivery: brain_store::ContextDelivery {
            project_id: binding.project_id,
            harness: envelope.harness.clone(),
            native_session_id: envelope
                .payload
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            event_name: envelope.event_name.to_string(),
            delivered_at: envelope.received_at,
            total_tokens,
            memory_tokens: 0,
            coordination_tokens: total_tokens,
            citation_count: 0,
        },
    })
}

/// A delivery metric awaiting proof that the reply arrived.
///
/// It carries its own ledger path rather than borrowing a handle, so the recording can happen on
/// whichever thread and at whichever moment the caller establishes delivery — which for the pipe
/// server is a blocking task after `flush()` returns.
pub struct PendingDelivery {
    ledger_path: PathBuf,
    project_id: ProjectId,
    delivery: brain_store::ContextDelivery,
}

impl PendingDelivery {
    /// Record it. Call only once the reply has actually reached the client.
    ///
    /// Fail-open by contract at every call site: losing a metric must never cost a session its
    /// orientation, and by the time this runs the orientation has already been delivered anyway.
    pub fn record(self) -> Result<()> {
        let ledger = EventLedger::open(&self.ledger_path, self.project_id)?;
        ledger.record_context_delivery(&self.delivery)
    }
}

pub struct ProjectHookHandler {
    bindings: Vec<ResolvedHookBinding>,
}

struct ResolvedHookBinding {
    binding: HookProjectBinding,
    normalized_root: String,
}

/// Backwards-compatible foundation name. New integrations should use
/// `ProjectHookHandler` because project boundaries and context are shared.
pub type ClaudeHookHandler = ProjectHookHandler;

impl ProjectHookHandler {
    pub fn new(binding: HookProjectBinding) -> Result<Self> {
        Self::for_projects(vec![binding])
    }

    pub fn for_projects(bindings: Vec<HookProjectBinding>) -> Result<Self> {
        let mut resolved = Vec::with_capacity(bindings.len());
        for mut binding in bindings {
            binding.project_root = canonical_project_root(&binding.project_root)?;
            resolved.push(ResolvedHookBinding {
                normalized_root: normalize_path(&binding.project_root),
                binding,
            });
        }
        resolved.sort_by_key(|binding| std::cmp::Reverse(binding.normalized_root.len()));
        Ok(Self { bindings: resolved })
    }

    /// Read every project's ledger once, so the first session start does not pay for a cold cache.
    ///
    /// A restart empties the OS file cache for these databases and leaves a WAL to recover, and the
    /// first orientation absorbs both: measured 10 August, `open` 2,172 ms and `load` 3,471 ms for
    /// a total of 6,571 ms against a 3 s hook budget. The session that triggered it got nothing —
    /// which is the worst possible moment to be slow, because a service restart is usually followed
    /// within seconds by the session starts that restarted it.
    ///
    /// The same reads, run here, cost the same time against nobody's session. The connection is
    /// closed straight after; what survives is the OS page cache and the recovered WAL, which is
    /// the part that was expensive.
    ///
    /// Fail-open and advisory: a project that cannot be warmed is a project whose first session
    /// start is merely as slow as it is today, so this warns and moves to the next one.
    pub fn warm(&self) {
        for resolved in &self.bindings {
            let binding = &resolved.binding;
            let started = std::time::Instant::now();
            let warmed =
                EventLedger::open(&binding.ledger_path, binding.project_id).and_then(|ledger| {
                    // The same two reads `ContextCompiler::from_ledger` makes, at the same limit —
                    // warming anything else would be warming pages the hook path does not touch.
                    ledger.recent_events(binding.project_id, ORIENTATION_EVENT_LIMIT)?;
                    ledger.current_project_memories()?;
                    Ok(())
                });
            match warmed {
                Ok(()) => tracing::info!(
                    project = %binding.project_id.0,
                    warm_ms = started.elapsed().as_millis(),
                    "ledger warmed"
                ),
                Err(error) => tracing::warn!(
                    %error,
                    project = %binding.project_id.0,
                    "ledger warm-up failed; the first session start pays the cold cost"
                ),
            }
        }
    }

    /// Compile a reply, and stage the delivery metric without recording it.
    ///
    /// The split exists because the two things are not the same event. An orientation that was
    /// compiled has cost the service work; an orientation that was *received* has done the user
    /// good, and only the second is worth counting. Recording inside this function conflated them
    /// for as long as the metric existed, and the gap is not hypothetical — one day's log held ten
    /// `write hook reply` failures with nine requests still spooled, every one of them counted as
    /// a delivery.
    ///
    /// The caller owns the second half: write the reply, flush it, and only then call
    /// [`PendingDelivery::record`].
    pub fn handle(&self, envelope: &HookEnvelope) -> Result<HookOutcome> {
        if !matches!(
            envelope.harness,
            Harness::ClaudeCode | Harness::Codex | Harness::Hermes
        ) {
            return Ok(HookOutcome::bare(HookReply::default()));
        }
        let Some(cwd) = envelope
            .payload
            .get("cwd")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(HookOutcome::bare(HookReply::default()));
        };
        let Ok(canonical_cwd) = std::fs::canonicalize(cwd) else {
            return Ok(HookOutcome::bare(HookReply::default()));
        };
        let normalized_cwd = normalize_path(&canonical_cwd);
        let Some(binding) = self.resolve_binding(&normalized_cwd) else {
            return Ok(HookOutcome::bare(HookReply::default()));
        };
        let lease_warning = manage_lease_lifecycle(&binding, envelope)?;
        if envelope.event_name == "UserPromptSubmit" {
            // Fail-open, and silent when there is nothing worth saying. A push that fires on every
            // prompt must be willing to return nothing far more often than it returns something.
            let pushed = match mid_session_push(&binding, envelope) {
                Ok(text) => text,
                Err(error) => {
                    tracing::warn!(%error, "mid-session push failed; the session continues without it");
                    None
                }
            };
            let additional_context = [lease_warning, pushed]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let additional_context = (!additional_context.is_empty()).then(|| {
                additional_context.join(
                    "

",
                )
            });
            let delivery = additional_context
                .as_deref()
                .and_then(|text| staged_push(&binding, envelope, text));
            return Ok(HookOutcome {
                reply: HookReply {
                    additional_context,
                    diagnostics_id: Some(envelope.nonce.to_string()),
                },
                delivery,
            });
        }
        if envelope.event_name == "SessionEnd" {
            // Fail-open. A session ending is not a moment to return an error to the harness, and
            // the cost of missing one is a session that consolidates on the next threshold instead
            // of immediately — not lost evidence, which the transcript still holds either way.
            if let Err(error) = record_session_end(&binding, envelope) {
                tracing::warn!(%error, "could not record session end");
            }
            let delivery = lease_warning
                .as_deref()
                .and_then(|text| staged_push(&binding, envelope, text));
            return Ok(HookOutcome {
                reply: HookReply {
                    additional_context: lease_warning,
                    diagnostics_id: Some(envelope.nonce.to_string()),
                },
                delivery,
            });
        }
        if envelope.event_name != "SessionStart" {
            return Ok(HookOutcome::bare(HookReply {
                additional_context: lease_warning,
                diagnostics_id: Some(envelope.nonce.to_string()),
            }));
        }

        // Timed in four stages, and the reason is that none of this was observable. The hook's
        // ceiling went from correct to wrong silently once, and diagnosing the second time meant
        // driving the named pipe by hand from three different languages because no CLI compiles an
        // orientation and nothing logged how long one took. A stage that is slow should say so in
        // the log the next person already reads.
        let started = std::time::Instant::now();
        let ledger = EventLedger::open(&binding.ledger_path, binding.project_id)?;
        let opened_ms = started.elapsed().as_millis();
        let live_state = LiveState::inspect(&binding.project_root, binding.worktree_id);
        let live_ms = started.elapsed().as_millis() - opened_ms;
        let mut compiler =
            ContextCompiler::from_ledger(&ledger, binding.project_id, ORIENTATION_EVENT_LIMIT)?
                .with_live_state(live_state);
        let loaded_ms = started.elapsed().as_millis() - opened_ms - live_ms;
        if let Some(path) = binding
            .global_preferences_path
            .as_ref()
            .filter(|path| path.is_file())
        {
            let preferences =
                brain_store::GlobalPreferenceStore::open(path)?.current_preferences()?;
            compiler = compiler.with_global_preferences(preferences);
        }
        if let Ok(results) = cached_wiki_context(&binding, envelope.received_at)
            && !results.is_empty()
        {
            compiler = compiler.with_provider_results(results);
        }
        let mut query = ContextQuery::for_worktree(binding.project_id, binding.worktree_id);
        let coordination = coordination_context(&binding, envelope.received_at)
            .ok()
            .flatten();
        if coordination.is_some() {
            query.max_tokens = 1_000;
        }
        let native_session_id = envelope
            .payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        query.native_session_id = native_session_id.clone();
        let before_compile = started.elapsed().as_millis();
        let compiled = compiler.compile(query)?;
        let compile_ms = started.elapsed().as_millis() - before_compile;
        // `info`, not `debug`. The whole point is that it is present in the log somebody reads
        // when a session starts slowly, without anyone having to raise a level first.
        tracing::info!(
            project = %binding.project_id.0,
            open_ms = opened_ms,
            live_state_ms = live_ms,
            load_ms = loaded_ms,
            compile_ms,
            total_ms = started.elapsed().as_millis(),
            citations = compiled.citations.len(),
            "orientation compiled"
        );
        if compiled.citations.is_empty() && coordination.is_none() && lease_warning.is_none() {
            // Nothing was compiled, so there is nothing to have delivered.
            return Ok(HookOutcome::bare(HookReply::default()));
        }
        let coordination = [lease_warning, coordination]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
        // Measured before `compiled.text` is consumed below.
        let memory_tokens = token_count(&compiled.text) as u64;
        let coordination_tokens = token_count(&coordination) as u64;
        let citation_count = compiled.citations.len() as u64;
        let additional_context = if coordination.is_empty() {
            compiled.text
        } else {
            format!("{coordination}\n\n{}", compiled.text)
        };
        debug_assert!(token_count(&additional_context) <= 1_500);
        // Staged, not recorded. Compiling an orientation is not delivering one, and this used to
        // record here — one step before the reply is written to the pipe — so every reply that
        // failed to reach its client still counted. The caller records it once the client has it.
        let delivery = PendingDelivery {
            ledger_path: binding.ledger_path.clone(),
            project_id: binding.project_id,
            delivery: brain_store::ContextDelivery {
                project_id: binding.project_id,
                harness: envelope.harness.clone(),
                native_session_id,
                event_name: envelope.event_name.to_string(),
                delivered_at: envelope.received_at,
                total_tokens: token_count(&additional_context) as u64,
                memory_tokens,
                coordination_tokens,
                citation_count,
            },
        };
        Ok(HookOutcome {
            reply: HookReply {
                additional_context: Some(additional_context),
                diagnostics_id: Some(envelope.nonce.to_string()),
            },
            delivery: Some(delivery),
        })
    }

    fn resolve_binding(&self, normalized_cwd: &str) -> Option<HookProjectBinding> {
        if let Some(resolved) = self
            .bindings
            .iter()
            .find(|binding| is_within_root(normalized_cwd, &binding.normalized_root))
        {
            return Some(resolved.binding.clone());
        }
        for resolved in &self.bindings {
            let Ok(store) =
                CoordinationStore::open(&resolved.binding.ledger_path, resolved.binding.project_id)
            else {
                continue;
            };
            let Ok(tasks) = store.tasks(false) else {
                continue;
            };
            for task in tasks {
                let Some(path) = task.worktree_path.as_deref() else {
                    continue;
                };
                let resolved_path =
                    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                let normalized_task = normalize_path(&resolved_path);
                if is_within_root(normalized_cwd, &normalized_task) {
                    let mut binding = resolved.binding.clone();
                    binding.project_root = resolved_path;
                    binding.worktree_id = task.worktree_id;
                    return Some(binding);
                }
            }
        }
        None
    }
}

fn cached_wiki_context(
    binding: &HookProjectBinding,
    now: time::OffsetDateTime,
) -> Result<Vec<ProviderResult>> {
    let project_storage = binding
        .ledger_path
        .parent()
        .context("project ledger has no storage directory")?;
    let brain_home = project_storage
        .parent()
        .and_then(Path::parent)
        .context("project ledger is not under BRAIN_HOME/projects/<id>")?;
    let config = ProviderConfig::load(project_storage.join("providers.json"), brain_home)?;
    if !config.llm_wiki.enabled {
        return Ok(Vec::new());
    }
    let config_hash = config.sha256()?;
    let cache = ProviderCacheStore::open(&binding.ledger_path, binding.project_id)?;
    let Some(entry) = cache.latest("llm_wiki", config_hash, now)? else {
        return Ok(Vec::new());
    };
    let age = (now - entry.fetched_at).whole_seconds().max(0);
    let mut items = serde_json::from_value::<Vec<ProviderResult>>(entry.items)?;
    items.retain(|item| {
        item.project_id == binding.project_id
            && item.worktree_id.is_none_or(|id| id == binding.worktree_id)
            && !item.source_uri.trim().is_empty()
    });
    items.truncate(2);
    for item in &mut items {
        item.trust = format!("external_document_cached age={age}s");
    }
    Ok(items)
}

/// Tokens a single mid-session push may spend.
///
/// **Far smaller than the session-start budget on purpose.** The orientation fires once and gets
/// 1,000–1,500 tokens; this fires on *every prompt*, so the same generosity would multiply by the
/// length of the conversation. Four hundred buys three or four cited lines, which is the size of
/// "you have seen this before" — not the size of a briefing.
const MID_SESSION_TOKENS: usize = 400;

/// Most memories one push may carry, before the token budget is even consulted.
const MAX_PUSHED_MEMORIES: usize = 4;

/// Most memories one session may ever be handed by the push path.
///
/// The no-repeat rule is per *memory*, not per prompt, so asking the same question twice correctly
/// surfaces the *next* four matches rather than the same four. Measured live: two identical prompts
/// pushed eight distinct memories. That is the right behaviour and an unbounded one — over a long
/// session it drains the corpus into context a few hundred tokens at a time, which is precisely the
/// drift the session-start budget exists to prevent.
///
/// Twenty is roughly five pushes. Past that the session has had a fair share and the honest answer
/// is silence; anything still missing can be asked for.
const MAX_SESSION_PUSHES: usize = 20;

/// Content terms a memory must share with the prompt before it may be pushed.
///
/// **A push needs a stricter floor than a search does**, because the user did not ask for it. FTS
/// terms are OR-joined, so a query shares "is", "the", "of" with almost everything and the ranking
/// dutifully returns *something* for any input — measured: a prompt about unladen swallows retrieved
/// a database-migration memory and would have injected it.
///
/// This makes the push deliberately conservative. It will miss a memory that is genuinely relevant
/// but shares no vocabulary — the same vocabulary gap the cross-encoder failed to close and query
/// expansion is meant to. Missing one is the right side to err on: an unsolicited wrong answer costs
/// more attention than a missing right one, and the user can always ask.
const MIN_SHARED_TERMS: usize = 2;

/// Shortest prompt worth searching on.
///
/// "ok", "continue", "yes" retrieve noise: they share no vocabulary with anything specific, so the
/// fused ranking returns whatever is generally popular. Below this length the honest push is none.
const MIN_PROMPT_CHARACTERS: usize = 24;

/// Re-orient a running session when the subject moves.
///
/// The gap this closes: the brain used to push **once**, at session start. A session that ran for
/// hours and pivoted to a different subject was never re-oriented — what arrived at minute zero was
/// all it ever got, and nothing about that failure was visible because both halves worked.
///
/// Three rules keep it from becoming noise:
///
/// 1. **Never repeat.** A memory reaches a given session at most once; `session_pushes` is the
///    record. Restating what the model already has is the fastest way to make a budget worthless.
/// 2. **Silence is a valid answer**, and the common one. A short prompt, no new memory, or nothing
///    above the floor all return `None` rather than something.
/// 3. **Always meter.** Every push ends with what it cost and how many memories it carried, because
///    a per-message injection that nobody can measure is exactly how a token contract stops holding.
fn mid_session_push(
    binding: &HookProjectBinding,
    envelope: &HookEnvelope,
) -> Result<Option<String>> {
    let Some(prompt) = envelope
        .payload
        .get("prompt")
        .or_else(|| envelope.payload.get("user_prompt"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| text.chars().count() >= MIN_PROMPT_CHARACTERS)
    else {
        return Ok(None);
    };
    let Some(session_id) = envelope
        .payload
        .get("session_id")
        .or_else(|| envelope.payload.get("sessionId"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.trim().is_empty())
    else {
        // With no session id there is no way to avoid repeating ourselves, and a push that repeats
        // is worse than no push.
        return Ok(None);
    };

    let mut ledger = EventLedger::open(&binding.ledger_path, binding.project_id)?;
    // The vector channel is what makes this work at all: a pivot is precisely the case where the
    // new subject shares no vocabulary with the session-start orientation.
    ledger.enable_vector_search(&binding.brain_home);

    let already = ledger.session_pushed_ids(session_id)?;
    if already.len() >= MAX_SESSION_PUSHES {
        return Ok(None);
    }
    let room = MAX_PUSHED_MEMORIES.min(MAX_SESSION_PUSHES - already.len());
    let hits = ledger.search(
        &brain_store::SearchQuery::text(binding.project_id, prompt)
            .memories_only()
            .with_limit(MAX_PUSHED_MEMORIES + already.len().min(24) + 4),
    )?;

    let mut lines = Vec::new();
    let mut pushed_ids = Vec::new();
    let mut dropped = 0_usize;
    let mut spent = 0_usize;
    for hit in hits {
        let Some(memory_id) = hit.memory_id else {
            continue;
        };
        if already.contains(&memory_id) {
            continue;
        }
        if shared_terms(prompt, &hit.title, &hit.text) < MIN_SHARED_TERMS {
            // Below the floor, and not counted as dropped: it was never a candidate, so reporting
            // it would overstate what the budget cost.
            continue;
        }
        if pushed_ids.len() >= room {
            dropped += 1;
            continue;
        }
        let line = format!(
            "- {} — {} (memory:{memory_id})",
            hit.title,
            first_sentence(&hit.text)
        );
        let cost = token_count(&line);
        if spent + cost > MID_SESSION_TOKENS {
            dropped += 1;
            continue;
        }
        spent += cost;
        lines.push(line);
        pushed_ids.push(memory_id);
    }

    if lines.is_empty() {
        return Ok(None);
    }

    // The meter is the last line, always, and it names what it dropped. A silent loss is worse than
    // the bloat it was trying to avoid.
    let meter = if dropped == 0 {
        format!(
            "[brain · {} {} · {spent} tokens]",
            lines.len(),
            if lines.len() == 1 {
                "memory"
            } else {
                "memories"
            }
        )
    } else {
        format!(
            "[brain · {} of {} memories · {spent} tokens · {dropped} dropped over budget]",
            lines.len(),
            lines.len() + dropped
        )
    };
    let body = format!(
        "Related memory for this turn:
{}
{meter}",
        lines.join(
            "
"
        )
    );

    // Recorded only once the text is built, so a push that failed to render records nothing.
    ledger.record_session_push(session_id, &pushed_ids, envelope.received_at)?;
    Ok(Some(body))
}

/// How many content-bearing terms the prompt and a memory have in common.
///
/// Length four and up, lower-cased, deduplicated. Crude on purpose: this is a floor that stops
/// nonsense, not a ranking — the ranking already happened.
fn shared_terms(prompt: &str, title: &str, body: &str) -> usize {
    fn terms(text: &str) -> std::collections::HashSet<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|word| word.chars().count() >= 4)
            .map(str::to_lowercase)
            .collect()
    }
    let asked = terms(prompt);
    let held = terms(&format!("{title} {body}"));
    asked.intersection(&held).count()
}

/// The first sentence of a memory's body, bounded — a push is a pointer, not the note.
fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    let head = trimmed
        .split_once(". ")
        .map(|(head, _)| head)
        .unwrap_or(trimmed);
    if head.chars().count() <= 160 {
        return head.to_owned();
    }
    let clipped: String = head.chars().take(159).collect();
    format!("{clipped}…")
}

/// Mark a session as ended, and consolidate what it produced.
///
/// **Nothing else can observe this boundary.** Transcripts are append-only JSONL: a session ending
/// writes no line, the file simply stops growing. So `EventType::SessionEnded` was never emitted
/// once — zero across 139,192 captured events in three projects — and
/// `ConsolidationReason::SessionStopped` was unreachable code that looked wired up. Sessions
/// consolidated only when they crossed the 200-event threshold, which means a short session's work
/// waited for the *next* session to push it over the line.
///
/// Idempotent by construction: the idempotency key is derived from the session id, so a harness
/// that fires `SessionEnd` twice appends once. That matters more than it sounds — `SessionEnd` has
/// several triggers (`clear`, `logout`, `prompt_input_exit`, `other`) and nothing promises exactly
/// one of them per session.
fn record_session_end(binding: &HookProjectBinding, envelope: &HookEnvelope) -> Result<()> {
    let Some(session_id) = envelope
        .payload
        .get("session_id")
        .or_else(|| envelope.payload.get("sessionId"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.trim().is_empty())
    else {
        // Without a session id there is nothing to attribute the boundary to, and inventing one
        // would create an episode that never existed.
        return Ok(());
    };
    let reason = envelope
        .payload
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("other");

    let payload = serde_json::json!({
        "session_id": session_id,
        "reason": reason,
        "harness": envelope.harness.as_str(),
    });
    let idempotency_key: [u8; 32] =
        sha2::Sha256::digest([b"session-end:".as_slice(), session_id.as_bytes()].concat()).into();

    let mut ledger = EventLedger::open(&binding.ledger_path, binding.project_id)?;
    let event_id = uuid::Uuid::now_v7();
    let result = ledger.append_batch(&brain_domain::EventBatch {
        // Its own source, so this can never disturb a transcript cursor. Cursors are keyed by
        // source and reusing a transcript's would re-ingest captured evidence.
        source_id: format!("hook-session-end:{session_id}"),
        events: vec![brain_domain::NormalizedEvent {
            event_id,
            project_id: binding.project_id,
            worktree_id: binding.worktree_id,
            task_id: None,
            harness: envelope.harness.clone(),
            native_session_id: session_id.to_owned(),
            native_turn_id: None,
            event_type: brain_domain::EventType::SessionEnded,
            occurred_at: envelope.received_at,
            observed_at: envelope.received_at,
            source_locator: format!("hook://session-end/{session_id}"),
            source_offset: 1,
            source_schema: "brain-session-end:v1".to_owned(),
            raw_hash: idempotency_key,
            idempotency_key,
            git_head: None,
            git_branch: None,
            payload: payload.clone(),
            raw: payload,
        }],
        quarantined: Vec::new(),
        capture_gaps: Vec::new(),
        next_cursor: brain_domain::SourceCursor::byte_offset(1),
    })?;
    if result.inserted == 0 {
        // Already recorded. Enqueueing again would consolidate the same span twice.
        return Ok(());
    }
    // The same bounded helper capture uses, rather than a second span calculation: it respects
    // `MAX_JOB_EVENTS` and `MAX_JOB_PAYLOAD_BYTES`, and the one time those bounds were bypassed a
    // single job covered 65,000 events.
    ledger.enqueue_through_event_job(event_id, brain_store::ConsolidationReason::SessionStopped)?;
    Ok(())
}

fn manage_lease_lifecycle(
    binding: &HookProjectBinding,
    envelope: &HookEnvelope,
) -> Result<Option<String>> {
    let Some(task_id) = envelope
        .payload
        .get("brain_task_id")
        .and_then(serde_json::Value::as_str)
        .map(uuid::Uuid::parse_str)
        .transpose()?
    else {
        return Ok(None);
    };
    let Some(native_session_id) = envelope
        .payload
        .get("session_id")
        .or_else(|| envelope.payload.get("sessionId"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(Some(
            "Writer lease was not acquired because the native session ID is missing.".to_owned(),
        ));
    };
    let owner = SessionIdentity {
        harness: envelope.harness.clone(),
        native_session_id: native_session_id.to_owned(),
    };
    let mut store = CoordinationStore::open(&binding.ledger_path, binding.project_id)?;
    let Some(task) = store.task(task_id)? else {
        return Ok(Some(format!(
            "Writer lease was not acquired because task {task_id} is not an active project task."
        )));
    };
    if task.worktree_id != binding.worktree_id {
        return Ok(Some(format!(
            "Writer lease was not acquired: task {task_id} belongs to worktree {}, but this session is in worktree {}. Switch to the task worktree before editing.",
            task.worktree_id.0, binding.worktree_id.0
        )));
    }
    let now = envelope.received_at;
    let result = match envelope.event_name.as_str() {
        "SessionEnd" | "session.end" => {
            let Some(lease) = store.lease(task_id)? else {
                return Ok(None);
            };
            store
                .release_lease(task_id, &owner, lease.generation, now)
                .map(|_| None)
        }
        "SessionStart" => store.acquire_lease(task_id, owner, now, None).map(|_| None),
        _ => match store.lease(task_id)? {
            Some(lease)
                if lease.owner == owner
                    && lease.renewed_at + RENEWAL_INTERVAL > now
                    && lease.expires_at > now =>
            {
                return Ok(None);
            }
            Some(lease) if lease.owner == owner && lease.expires_at > now => store
                .renew_lease(task_id, &owner, lease.generation, now, None)
                .map(|_| None),
            _ => store.acquire_lease(task_id, owner, now, None).map(|_| None),
        },
    };
    match result {
        Ok(value) => Ok(value),
        Err(LeaseError::AlreadyHeld {
            owner_harness,
            owner_session,
            expires_at,
        }) => Ok(Some(format!(
            "WARNING: writer lease is held by {owner_harness}/{owner_session} until {expires_at}. Do not edit in this worktree; create or switch to a separate task worktree, or perform an explicit handoff."
        ))),
        Err(error) => Ok(Some(format!("Writer lease update failed: {error}"))),
    }
}

pub(crate) fn coordination_context(
    binding: &HookProjectBinding,
    now: time::OffsetDateTime,
) -> Result<Option<String>> {
    let store = CoordinationStore::open(&binding.ledger_path, binding.project_id)?;
    let tasks = store.tasks(false)?;
    if tasks.is_empty() {
        return Ok(Some(
            "Coordination state: no active task is selected. Before editing, create or select a brain task and use its separate worktree; do not assume writer ownership."
                .to_owned(),
        ));
    }
    let leases = store.active_leases(now)?;
    let claims = store.active_claims()?;
    let mut lines =
        vec!["Coordination state (current project-scoped leases and claims):".to_owned()];
    for task in &tasks {
        let owner = leases
            .iter()
            .find(|lease| lease.task_id == task.id)
            .map(|lease| {
                format!(
                    "{} / {} until {} (generation {})",
                    lease.owner.harness.as_str(),
                    lease.owner.native_session_id,
                    lease.expires_at,
                    lease.generation
                )
            })
            .unwrap_or_else(|| "no active writer lease".to_owned());
        let current = if task.worktree_id == binding.worktree_id {
            " [current worktree]"
        } else {
            ""
        };
        lines.push(format!(
            "- task {} {:?}{}; owner={owner}",
            task.id, task.title, current
        ));
    }
    for claim in &claims {
        lines.push(format!(
            "- claim task={} kind={:?} path={}",
            claim.task_id, claim.kind, claim.display_value
        ));
    }
    for (index, left) in claims.iter().enumerate() {
        for right in claims.iter().skip(index + 1) {
            if left.task_id == right.task_id {
                continue;
            }
            let level = overlap(left, right);
            if matches!(
                level,
                Overlap::Definite | Overlap::Probable | Overlap::Possible
            ) {
                lines.push(format!(
                    "- OVERLAP {level:?}: task {} {} <> task {} {}",
                    left.task_id, left.display_value, right.task_id, right.display_value
                ));
            }
        }
    }
    Ok(Some(bound_tokens(&lines.join("\n"), 350)))
}

fn bound_tokens(value: &str, max_tokens: usize) -> String {
    if token_count(value) <= max_tokens {
        return value.to_owned();
    }
    let mut words = value.split_whitespace().collect::<Vec<_>>();
    while !words.is_empty() && token_count(&words.join(" ")) > max_tokens.saturating_sub(10) {
        words.pop();
    }
    words.join(" ") + " ... [coordination truncated]"
}

fn canonical_project_root(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path)
        .with_context(|| format!("resolve hook project root {}", path.display()))
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn is_within_root(candidate: &str, root: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}
