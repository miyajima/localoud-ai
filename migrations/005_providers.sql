CREATE TABLE provider_profiles(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    protocol TEXT NOT NULL,
    base_url TEXT,
    locality TEXT NOT NULL,
    credential_env TEXT,
    max_concurrency INTEGER NOT NULL CHECK(max_concurrency BETWEEN 1 AND 8),
    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
    revision INTEGER NOT NULL CHECK(revision >= 1),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE provider_segments(
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES provider_threads(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES provider_profiles(id),
    profile_revision INTEGER NOT NULL CHECK(profile_revision >= 1),
    model_id TEXT NOT NULL,
    effort TEXT,
    provider_thread_id TEXT,
    started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    ended_at TEXT
);

CREATE TABLE turn_model_targets(
    turn_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES provider_threads(id) ON DELETE CASCADE,
    segment_id TEXT NOT NULL REFERENCES provider_segments(id),
    profile_id TEXT NOT NULL REFERENCES provider_profiles(id),
    profile_revision INTEGER NOT NULL CHECK(profile_revision >= 1),
    model_id TEXT NOT NULL,
    effort TEXT,
    resolved_from TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE session_model_policies(
    thread_id TEXT PRIMARY KEY REFERENCES provider_threads(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE provider_messages(
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES provider_threads(id) ON DELETE CASCADE,
    segment_id TEXT REFERENCES provider_segments(id),
    provider_turn_id TEXT,
    role TEXT NOT NULL CHECK(role IN ('system','user','assistant','tool')),
    body TEXT NOT NULL,
    provider_state TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE tool_approvals(
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES provider_threads(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL,
    digest TEXT NOT NULL,
    request TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','approved','denied','consumed')),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    decided_at TEXT,
    consumed_at TEXT,
    UNIQUE(thread_id, turn_id, digest)
);

CREATE TABLE review_runs(
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    executor_thread_id TEXT NOT NULL REFERENCES provider_threads(id),
    reviewer_thread_id TEXT NOT NULL REFERENCES provider_threads(id),
    artifact_hash TEXT NOT NULL,
    verdict TEXT NOT NULL CHECK(verdict IN ('pending','pass','rework','inconclusive')),
    body TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TEXT,
    CHECK(executor_thread_id != reviewer_thread_id)
);

CREATE INDEX provider_segments_thread ON provider_segments(thread_id, started_at);
CREATE INDEX turn_model_targets_thread ON turn_model_targets(thread_id, created_at);
CREATE INDEX provider_messages_thread ON provider_messages(thread_id, created_at);
CREATE INDEX tool_approvals_turn ON tool_approvals(thread_id, turn_id, status);
CREATE INDEX review_runs_task ON review_runs(task_id, created_at);

INSERT INTO provider_profiles(id,name,protocol,base_url,locality,credential_env,max_concurrency,enabled,revision)
VALUES ('codex','Codex','codex_app_server',NULL,'local',NULL,1,1,1);

INSERT INTO provider_segments(id,thread_id,profile_id,profile_revision,model_id,provider_thread_id)
SELECT 'legacy-' || id,id,'codex',1,'',provider_thread_id
FROM provider_threads WHERE provider='codex';

PRAGMA user_version=5;
