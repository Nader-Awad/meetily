-- Multiple voice exemplars per speaker profile (multi-enrollment).
--
-- Previously each profile stored ONE averaged centroid (speaker_profiles.embedding),
-- which drifted and could be contaminated by a single mixed enrollment. We now keep
-- several raw exemplar embeddings per profile and match against the best-matching one.
-- speaker_profiles.embedding is retained as a maintained SUMMARY (mean of exemplars)
-- for display / backward compatibility and the raw-fallback matching path.
--
-- Additive & non-destructive: each existing profile's current centroid is backfilled
-- as its first exemplar, so nothing already trained is lost.
CREATE TABLE IF NOT EXISTS speaker_profile_embeddings (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    embedding BLOB NOT NULL,
    created_at TIMESTAMP NOT NULL,
    FOREIGN KEY (profile_id) REFERENCES speaker_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_speaker_profile_embeddings_profile
    ON speaker_profile_embeddings (profile_id);

-- Backfill: each existing profile's centroid becomes its first exemplar.
INSERT INTO speaker_profile_embeddings (id, profile_id, embedding, created_at)
SELECT lower(hex(randomblob(16))), id, embedding, created_at
FROM speaker_profiles;
