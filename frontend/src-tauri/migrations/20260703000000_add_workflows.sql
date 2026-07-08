-- Workflows: saved, named summary recipes (template/prompt + pinned model + export config)
CREATE TABLE IF NOT EXISTS workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    template_id TEXT NOT NULL,
    custom_prompt TEXT,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    max_tokens INTEGER,
    temperature REAL,
    top_p REAL,
    neohive_export TEXT,          -- JSON: NeoHiveExportConfig (null/absent = disabled)
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Workflow runs: retained artifacts. workflow_id is NOT a FK because runs survive
-- workflow deletion; workflow_name is a denormalized snapshot for display.
CREATE TABLE IF NOT EXISTS workflow_runs (
    id TEXT PRIMARY KEY,
    workflow_id TEXT,
    workflow_name TEXT NOT NULL,
    meeting_id TEXT NOT NULL,
    status TEXT NOT NULL,             -- queued | running | completed | error | cancelled
    result_markdown TEXT,
    result_sections TEXT,            -- JSON: [{ "title": .., "content": .. }]
    error TEXT,
    neohive_status TEXT NOT NULL DEFAULT 'none',  -- none | pushed | partial | failed
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workflow_runs_meeting ON workflow_runs(meeting_id);

-- NeoHive export settings (single-row settings table, id = '1')
ALTER TABLE settings ADD COLUMN neohiveEndpoint TEXT;
ALTER TABLE settings ADD COLUMN neohiveApiKey TEXT;
ALTER TABLE settings ADD COLUMN neohiveEnabled INTEGER NOT NULL DEFAULT 0;
