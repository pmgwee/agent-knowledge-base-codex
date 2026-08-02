use anyhow::{Context, Result};
use brain_domain::SourceCursor;
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) fn save_cursor(
    transaction: &Transaction<'_>,
    source_id: &str,
    cursor: &SourceCursor,
) -> Result<()> {
    let cursor_json = serde_json::to_string(cursor)?;
    let updated_at_ns = timestamp_ns(time::OffsetDateTime::now_utc())?;
    transaction.execute(
        r#"
        INSERT INTO source_cursors(source_id, cursor_json, updated_at_ns)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(source_id) DO UPDATE SET
            cursor_json = excluded.cursor_json,
            updated_at_ns = excluded.updated_at_ns
        "#,
        params![source_id, cursor_json, updated_at_ns],
    )?;
    Ok(())
}

pub(crate) fn load_cursor(
    connection: &rusqlite::Connection,
    source_id: &str,
) -> Result<SourceCursor> {
    let value: Option<String> = connection
        .query_row(
            "SELECT cursor_json FROM source_cursors WHERE source_id = ?1",
            [source_id],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|json| serde_json::from_str(&json).context("stored source cursor is invalid"))
        .transpose()
        .map(|cursor| cursor.unwrap_or_else(SourceCursor::start))
}

pub(crate) fn timestamp_ns(value: time::OffsetDateTime) -> Result<i64> {
    i64::try_from(value.unix_timestamp_nanos()).context("timestamp is outside SQLite range")
}
