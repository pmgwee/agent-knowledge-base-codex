//! What a live session has already been handed, so a re-orientation never repeats itself.
//!
//! The session-start orientation fires once and can afford to restate everything. A push that fires
//! on **every prompt** cannot. Re-injecting the same memory each time the subject stays put would
//! spend the whole budget on something the model already has — and it would do that most
//! aggressively exactly when the conversation is going well and the topic is not moving.
//!
//! So a push is scoped by session and by memory: each memory reaches a given session at most once.
//! A new session starts clean, because a new session has none of it in context.
//!
//! **This is deliberately not the same as access counting.** `memory_access` records what retrieval
//! *reached*, which is the input to decay. This records what was *delivered into a conversation*,
//! which is the input to not repeating yourself. Conflating them would make an unread push look like
//! a memory someone wanted.

use anyhow::Result;
use rusqlite::params;

use crate::cursor::timestamp_ns;
use crate::ledger::EventLedger;

impl EventLedger {
    /// Record that these memories were pushed into a session.
    ///
    /// Idempotent per (session, memory): pushing the same memory twice records once, so a retry or a
    /// double-fired hook cannot corrupt the "already seen" set.
    pub fn record_session_push(
        &self,
        native_session_id: &str,
        memory_ids: &[uuid::Uuid],
        now: time::OffsetDateTime,
    ) -> Result<()> {
        if memory_ids.is_empty() || native_session_id.trim().is_empty() {
            return Ok(());
        }
        let at = timestamp_ns(now)?;
        let project = self.project_scope.0.to_string();
        let mut statement = self.connection.prepare_cached(
            "INSERT INTO session_pushes(native_session_id, memory_id, project_id, pushed_at_ns)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(native_session_id, memory_id) DO NOTHING",
        )?;
        for memory_id in memory_ids {
            statement.execute(params![
                native_session_id,
                memory_id.to_string(),
                &project,
                at
            ])?;
        }
        Ok(())
    }

    /// Which memories this session has already been handed.
    pub fn session_pushed_ids(
        &self,
        native_session_id: &str,
    ) -> Result<std::collections::HashSet<uuid::Uuid>> {
        if native_session_id.trim().is_empty() {
            return Ok(Default::default());
        }
        let mut statement = self.connection.prepare_cached(
            "SELECT memory_id FROM session_pushes
             WHERE native_session_id = ?1 AND project_id = ?2",
        )?;
        let rows = statement.query_map(
            params![native_session_id, self.project_scope.0.to_string()],
            |row| row.get::<_, String>(0),
        )?;
        rows.map(|row| Ok(uuid::Uuid::parse_str(&row?)?)).collect()
    }

    /// How many pushes a session has received. Reported so a runaway hook is visible.
    pub fn session_push_count(&self, native_session_id: &str) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM session_pushes WHERE native_session_id = ?1 AND project_id = ?2",
            params![native_session_id, self.project_scope.0.to_string()],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
