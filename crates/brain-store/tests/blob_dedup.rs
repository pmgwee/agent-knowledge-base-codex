use brain_domain::ProjectId;
use brain_store::{BlobStore, EventLedger};

#[test]
fn identical_large_payloads_share_one_blob_with_separate_project_references() {
    let temp = tempfile::tempdir().expect("temp");
    let brain_home = temp.path().join("brain");
    let project_a = ProjectId(uuid::Uuid::now_v7());
    let project_b = ProjectId(uuid::Uuid::now_v7());
    let ledger_a = temp.path().join("a.sqlite");
    let ledger_b = temp.path().join("b.sqlite");
    EventLedger::open(&ledger_a, project_a).expect("ledger A");
    EventLedger::open(&ledger_b, project_b).expect("ledger B");
    let store = BlobStore::new(&brain_home).expect("blob store");
    let payload = vec![b'x'; 128 * 1024];

    let first = store
        .put(&ledger_a, project_a, "application/octet-stream", &payload)
        .expect("first blob");
    let second = store
        .put(&ledger_b, project_b, "application/octet-stream", &payload)
        .expect("second blob");

    assert_eq!(first.raw_sha256, second.raw_sha256);
    assert_eq!(first.path, second.path);
    assert_eq!(store.physical_count().expect("physical count"), 1);
    assert_eq!(store.read(&first).expect("read blob"), payload);
    assert_eq!(reference_count(&ledger_a, project_a), 1);
    assert_eq!(reference_count(&ledger_b, project_b), 1);
}

fn reference_count(path: &std::path::Path, project: ProjectId) -> u64 {
    let connection = rusqlite::Connection::open(path).expect("connection");
    connection
        .query_row(
            "SELECT COUNT(*) FROM project_blob_refs WHERE project_id = ?1",
            [project.0.to_string()],
            |row| row.get(0),
        )
        .expect("reference count")
}
