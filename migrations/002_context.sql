ALTER TABLE context_capsules ADD COLUMN initial_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE context_retrievals ADD COLUMN result_json TEXT NOT NULL DEFAULT '{}';
CREATE UNIQUE INDEX context_capsule_task ON context_capsules(task_id);
CREATE INDEX task_project ON tasks(project_id);
CREATE INDEX retrieval_task ON context_retrievals(task_id);
PRAGMA user_version=2;
