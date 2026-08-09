//! Replay projections — a session as a UI can actually fetch it, and one cited turn in full.
//!
//! `EventLedger::session_events` returns `StoredEvent`, which carries the original `raw` record
//! beside the normalised one. That is correct for the ledger and wrong for a wire: this project's
//! largest captured session is 15,969 events and serialises to **100 MB**, so a replay view that
//! fetched a session would download a tenth of a gigabyte to render twenty turns.
//!
//! So the list view gets a projection — enough to render a turn and decide whether to open it —
//! and opening one fetches that single event whole. Which is also exactly the shape a citation
//! needs: `event:<uuid>` appears throughout the orientation and the search results, and expanding
//! one is a single-event fetch, not a session download.

use anyhow::Result;
use brain_domain::ProjectId;
use brain_store::{EventLedger, StoredEvent};

/// How much of a turn the list view carries.
///
/// Long enough to recognise the turn, short enough that a full page of them stays under the size a
/// list is worth fetching. A turn longer than this is opened, not skimmed.
const PREVIEW_CHARACTERS: usize = 280;

/// The default page. Chosen against the median session rather than the largest — most captured
/// sessions here are under 200 events, so this is one fetch for most of them and a pager for the
/// rest.
pub const DEFAULT_PAGE: usize = 100;

#[derive(Clone, Debug, serde::Serialize)]
pub struct ReplaySession {
    pub native_session_id: String,
    pub event_count: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub last_event_at: time::OffsetDateTime,
}

/// One turn, as a list renders it.
///
/// `preview` is truncated and `truncated` says so, rather than leaving a caller to guess from the
/// length whether it is looking at a whole turn or the start of one.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReplayTurn {
    pub event_id: uuid::Uuid,
    pub event_type: String,
    pub harness: String,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: time::OffsetDateTime,
    pub preview: String,
    pub characters: usize,
    pub truncated: bool,
}

/// One window of a session.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReplayPage {
    pub native_session_id: String,
    /// The whole session, not the page — a pager that cannot say "of how many" is a scroll bar
    /// with no thumb.
    pub total_events: u64,
    pub offset: usize,
    pub limit: usize,
    pub turns: Vec<ReplayTurn>,
}

/// One event in full, for expanding a citation in place.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReplayEvent {
    pub event_id: uuid::Uuid,
    pub native_session_id: String,
    pub event_type: String,
    pub harness: String,
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: time::OffsetDateTime,
    pub source_locator: String,
    pub git_branch: Option<String>,
    pub git_head: Option<String>,
    /// The turn's text if the payload holds one, else `None` — a tool call and a user message are
    /// both events, and only one of them has something to read.
    pub content: Option<String>,
    /// The normalised payload. Not `raw`: `raw` is the same information in the harness's own shape,
    /// and shipping both doubles the response to say one thing twice.
    pub payload: serde_json::Value,
}

/// The sessions a project has captured, newest first.
pub fn sessions(ledger: &EventLedger, limit: usize) -> Result<Vec<ReplaySession>> {
    Ok(ledger
        .captured_sessions(limit)?
        .into_iter()
        .map(
            |(native_session_id, event_count, last_event_at)| ReplaySession {
                native_session_id,
                event_count,
                last_event_at,
            },
        )
        .collect())
}

/// One window of one session.
pub fn page(
    ledger: &EventLedger,
    project_id: ProjectId,
    native_session_id: &str,
    limit: usize,
    offset: usize,
) -> Result<ReplayPage> {
    let events = ledger.session_events_page(project_id, native_session_id, limit, offset)?;
    Ok(ReplayPage {
        native_session_id: native_session_id.to_owned(),
        total_events: ledger.session_event_count(native_session_id)?,
        offset,
        limit,
        turns: events.iter().map(turn).collect(),
    })
}

/// One event in full, by id. `None` when this project's ledger does not hold it — which is the
/// honest answer for a citation pointing at another project, and never a cross-project read.
pub fn event(ledger: &EventLedger, event_id: uuid::Uuid) -> Result<Option<ReplayEvent>> {
    Ok(ledger.event(event_id)?.map(|event| ReplayEvent {
        event_id: event.event_id,
        native_session_id: event.native_session_id.clone(),
        event_type: event.event_type.as_str().to_owned(),
        harness: event.harness.as_str().to_owned(),
        occurred_at: event.occurred_at,
        source_locator: event.source_locator.clone(),
        git_branch: event.git_branch.clone(),
        git_head: event.git_head.clone(),
        content: content_of(&event),
        payload: event.payload,
    }))
}

fn turn(event: &StoredEvent) -> ReplayTurn {
    let text = content_of(event).unwrap_or_default();
    let characters = text.chars().count();
    let truncated = characters > PREVIEW_CHARACTERS;
    ReplayTurn {
        event_id: event.event_id,
        event_type: event.event_type.as_str().to_owned(),
        harness: event.harness.as_str().to_owned(),
        occurred_at: event.occurred_at,
        // By characters, not bytes. A `&str[..280]` panics mid-codepoint, and it would do so on
        // real transcripts rather than on a fixture.
        preview: text.chars().take(PREVIEW_CHARACTERS).collect(),
        characters,
        truncated,
    }
}

/// The readable text of an event, wherever the adapter put it.
///
/// There is no single field, and assuming there was is what made the first version of this render
/// blank rows: the 818 most recent `agent.responded` events in this project all carry their text
/// under `message.content` as a *block array*, so a `payload["content"].as_str()` returned `None`
/// for every one of them and the replay list showed timestamps beside nothing.
///
/// The shapes actually present, measured over 4,000 events across both harnesses:
///
/// | Where | Which events |
/// |---|---|
/// | `content` as a string | `tool.completed`, `tool.failed` |
/// | `content` as a block array | `attachment.observed` |
/// | `message.content` as a string | `user.prompted` (claude-code) |
/// | `message.content` as a block array | `agent.responded` |
/// | `message` as a string | `user.prompted` (codex) |
/// | `input` / `output` | `tool.requested` / `tool.completed` (codex) |
///
/// Anything else yields `None`, which renders as an event with no text rather than as a guess.
fn content_of(event: &StoredEvent) -> Option<String> {
    let payload = &event.payload;
    for key in ["content", "text", "message", "output", "input"] {
        if let Some(value) = payload.get(key)
            && let Some(text) = readable(value)
        {
            return Some(text);
        }
    }
    None
}

/// Text out of one JSON value, following the two nestings the adapters produce.
fn readable(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        serde_json::Value::Array(blocks) => {
            let joined = blocks
                .iter()
                .filter_map(block_text)
                .collect::<Vec<_>>()
                .join("\n");
            (!joined.trim().is_empty()).then_some(joined)
        }
        serde_json::Value::Object(map) => map
            .get("content")
            .or_else(|| map.get("text"))
            .or_else(|| map.get("command"))
            .and_then(readable),
        _ => None,
    }
}

/// One content block. Tool calls become a one-line marker rather than their whole input, and
/// thinking blocks contribute their text but never their `signature` — that field is an opaque
/// base64 blob, and rendering it would fill a preview with noise that looks like content.
fn block_text(block: &serde_json::Value) -> Option<String> {
    let map = block.as_object()?;
    match map.get("type").and_then(serde_json::Value::as_str) {
        Some("text") => map.get("text").and_then(readable),
        Some("thinking") => map.get("thinking").and_then(readable),
        Some("tool_use") => map
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(|name| format!("→ {name}")),
        Some("tool_result") => map.get("content").and_then(readable),
        // No `type`, or one we have not seen. Fall back to the same field order rather than
        // dropping the block: an unrecognised shape carrying text should still show its text.
        _ => map
            .get("text")
            .or_else(|| map.get("content"))
            .and_then(readable),
    }
}

/// The list view, as a terminal renders it.
pub fn render_page(page: &ReplayPage) -> String {
    let mut out = String::new();
    let last = page.offset + page.turns.len();
    out.push_str(&format!(
        "{} — {}–{} of {} events\n\n",
        page.native_session_id,
        page.offset + 1,
        last,
        page.total_events
    ));
    for turn in &page.turns {
        let preview = turn
            .preview
            .split_whitespace()
            .take(14)
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!(
            "  {}  {:<22} {preview}\n",
            turn.occurred_at.time(),
            turn.event_type
        ));
    }
    if last < page.total_events as usize {
        out.push_str(&format!(
            "\n  … {} more — --offset {last}\n",
            page.total_events as usize - last
        ));
    }
    out
}
