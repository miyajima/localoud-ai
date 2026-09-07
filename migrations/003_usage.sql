CREATE TABLE provider_usage_snapshots(
    id TEXT PRIMARY KEY,
    provider_thread_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    total_input_tokens INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    cached_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    UNIQUE(provider_thread_id,total_input_tokens)
);
PRAGMA user_version=3;
