-- Custom vocabulary: a two-entry-type dictionary (terms + corrections) with
-- optional per-term descriptions. JSON blob on the single-row settings table
-- (id = '1'; camelCase columns), mirroring customOpenAIConfig.
ALTER TABLE settings ADD COLUMN vocabularyConfig TEXT;
