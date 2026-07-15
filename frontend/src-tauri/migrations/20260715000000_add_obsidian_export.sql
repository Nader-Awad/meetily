-- Obsidian export: write completed workflow runs as markdown files to a vault folder.

-- Global connection settings (single-row settings table, id = '1'; camelCase columns).
ALTER TABLE settings ADD COLUMN obsidianVaultPath TEXT;
ALTER TABLE settings ADD COLUMN obsidianEnabled INTEGER NOT NULL DEFAULT 0;

-- Per-workflow Obsidian export config (JSON; null/absent = disabled). Sibling of neohive_export.
ALTER TABLE workflows ADD COLUMN obsidian_export TEXT;

-- Per-run Obsidian result, mirroring neohive_status.
ALTER TABLE workflow_runs ADD COLUMN obsidian_status TEXT NOT NULL DEFAULT 'none'; -- none | saved | failed
ALTER TABLE workflow_runs ADD COLUMN obsidian_path TEXT;                            -- absolute path last written
