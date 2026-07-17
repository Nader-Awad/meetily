-- Cloud STT: OpenRouter/custom API keys + a custom base URL, on the single-row transcript_settings table.
ALTER TABLE transcript_settings ADD COLUMN openrouterApiKey TEXT;
ALTER TABLE transcript_settings ADD COLUMN customApiKey TEXT;
ALTER TABLE transcript_settings ADD COLUMN transcriptBaseUrl TEXT;
