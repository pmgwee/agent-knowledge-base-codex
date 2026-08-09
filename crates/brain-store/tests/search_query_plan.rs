//! The memories keyword query must reach supersession by index, not by scanning versions.
//!
//! This pins a plan, not a timing, because the defect it guards was invisible in every other form.
//! `memory_supersession`'s primary key indexes it by the *superseding* version, while every read
//! asks the opposite question — "has this version been superseded?" With no index on that column
//! the answer is unreachable by lookup, so SQLite drove the subquery from `memory_versions`
//! instead, once per candidate row.
//!
//! Measured on the live 5,669-version ledger, on the query every session-start orientation runs:
//! **42,001 ms without the index, 34.1 ms with it**, identical results, index built in 3 ms. It
//! scales with versions × matched rows, so it stayed invisible while projects were small and then
//! consumed the entire 3 s hook budget on the largest — two of three registered projects delivered
//! no orientation at all.
//!
//! A timing assertion would be flaky and would say nothing on a fixture this size. The plan is the
//! actual invariant: reach `memory_supersession` by `superseded_version_id`, and never sweep
//! `memory_versions` to answer it.

use brain_domain::ProjectId;
use brain_store::EventLedger;

/// The supersession filter as `search_memories` writes it, reduced to the clause under test.
const SUPERSESSION_FILTER: &str = r#"
    SELECT v.version_id
    FROM memory_versions v
    WHERE NOT EXISTS (
        SELECT 1
        FROM memory_supersession s
        JOIN memory_versions newer ON newer.version_id = s.version_id
        WHERE s.superseded_version_id = v.version_id
          AND newer.valid_from_ns <= 1786300000000000000
    )
"#;

#[test]
fn the_supersession_filter_reaches_its_index_instead_of_scanning_versions() {
    let ledger = EventLedger::open_in_memory(ProjectId(uuid::Uuid::now_v7())).expect("ledger");
    let plan = ledger
        .explain_query_plan(SUPERSESSION_FILTER)
        .expect("plan the supersession filter");

    assert!(
        plan.iter()
            .any(|step| step.contains("idx_memory_supersession_superseded")),
        "the supersession filter no longer reaches its index — it will scan memory_versions once \
         per candidate row, which measured 42 s against 34 ms on the live ledger. Plan was:\n{}",
        plan.join("\n")
    );
    assert!(
        !plan
            .iter()
            .any(|step| step.contains("newer USING INDEX idx_memory_versions_validity")),
        "the subquery is being driven from memory_versions again, which is the shape that took \
         42 s. Plan was:\n{}",
        plan.join("\n")
    );
}
