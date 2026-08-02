CREATE TABLE schema_version (
    version INTEGER NOT NULL
);
INSERT INTO schema_version(version) VALUES (999);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    started_at REAL NOT NULL,
    cwd TEXT
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL,
    content TEXT,
    timestamp REAL NOT NULL,
    future_required_column TEXT NOT NULL
);
