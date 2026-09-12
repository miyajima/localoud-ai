CREATE TABLE provider_turn_outcomes(
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES provider_threads(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK(status IN ('completed','refused','failed','interrupted','unknown')),
    stop_reason TEXT,
    failure_kind TEXT,
    detail TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX provider_turn_outcomes_thread
ON provider_turn_outcomes(thread_id, created_at);

PRAGMA user_version=6;
