// database/repositories/speaker_profile.rs
//
// CRUD for persistent voice profiles (speaker identification).
// Embeddings are stored as f32 little-endian BLOBs.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct SpeakerProfileRow {
    pub id: String,
    pub name: String,
    pub embedding: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerProfile {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub embedding: Vec<f32>,
}

pub fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|v| v.to_le_bytes()).collect()
}

pub fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Running-mean accrual of a saved profile centroid toward a newly-confirmed
/// cluster centroid, then re-normalized. `prior_count` is how many segments the
/// existing centroid already represents (weight of the old value).
pub fn accrue_centroid(existing: &[f32], prior_count: usize, new: &[f32]) -> Vec<f32> {
    if existing.len() != new.len() || existing.is_empty() {
        return existing.to_vec();
    }
    let w = prior_count.max(1) as f32;
    let mut out: Vec<f32> = existing
        .iter()
        .zip(new.iter())
        .map(|(e, n)| (e * w + n) / (w + 1.0))
        .collect();
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut out {
            *x /= norm;
        }
    }
    out
}

/// Default cap on stored exemplars per profile. Enough to cover within-speaker
/// variation (different mics, energy, rooms) without bloating match cost.
pub const DEFAULT_MAX_EXEMPLARS: usize = 6;

/// A saved profile together with all of its stored raw voice exemplars.
#[derive(Debug, Clone)]
pub struct ProfileExemplars {
    pub id: String,
    pub name: String,
    pub exemplars: Vec<Vec<f32>>,
}

/// L2-normalized element-wise mean of equal-length embeddings — the "summary"
/// centroid kept on `speaker_profiles.embedding` for display and the raw
/// fallback path. Length-mismatched vectors are skipped; `None` if the input is
/// empty or the mean has zero norm.
pub fn mean_normalized(embeddings: &[Vec<f32>]) -> Option<Vec<f32>> {
    let dim = embeddings.iter().find(|e| !e.is_empty())?.len();
    let mut sum = vec![0.0f32; dim];
    let mut count = 0usize;
    for e in embeddings {
        if e.len() == dim {
            for (a, v) in sum.iter_mut().zip(e) {
                *a += v;
            }
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    for v in &mut sum {
        *v /= count as f32;
    }
    let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= 0.0 {
        return None;
    }
    for v in &mut sum {
        *v /= norm;
    }
    Some(sum)
}

/// When a profile exceeds its exemplar cap, pick which one to evict: the most
/// *redundant* exemplar — the one whose nearest neighbour among the others is
/// closest — so we shed a near-duplicate and keep diverse coverage rather than
/// blindly dropping the oldest. Assumes L2-normalized inputs. `None` if < 2.
pub fn most_redundant_exemplar_index(embeddings: &[Vec<f32>]) -> Option<usize> {
    if embeddings.len() < 2 {
        return None;
    }
    let cos = |a: &[f32], b: &[f32]| -> f32 {
        if a.len() != b.len() {
            return -1.0;
        }
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    };
    let mut worst: (usize, f32) = (0, f32::MIN);
    for (i, a) in embeddings.iter().enumerate() {
        let nearest = embeddings
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, b)| cos(a, b))
            .fold(f32::MIN, f32::max);
        if nearest > worst.1 {
            worst = (i, nearest);
        }
    }
    Some(worst.0)
}

pub struct SpeakerProfilesRepository;

impl SpeakerProfilesRepository {
    pub async fn list(pool: &SqlitePool) -> Result<Vec<SpeakerProfile>, SqlxError> {
        let rows = sqlx::query_as::<_, SpeakerProfileRow>(
            "SELECT id, name, embedding FROM speaker_profiles ORDER BY name",
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SpeakerProfile {
                id: r.id,
                name: r.name,
                embedding: blob_to_embedding(&r.embedding),
            })
            .collect())
    }

    pub async fn create(
        pool: &SqlitePool,
        name: &str,
        embedding: &[f32],
    ) -> Result<String, SqlxError> {
        let id = format!("speaker-{}", Uuid::new_v4());
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO speaker_profiles (id, name, embedding, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(embedding_to_blob(embedding))
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
        // Seed the profile's first exemplar (the summary starts equal to it).
        Self::insert_exemplar_row(pool, &id, embedding).await?;
        Ok(id)
    }

    pub async fn update_embedding(
        pool: &SqlitePool,
        id: &str,
        embedding: &[f32],
    ) -> Result<(), SqlxError> {
        sqlx::query("UPDATE speaker_profiles SET embedding = ?, updated_at = ? WHERE id = ?")
            .bind(embedding_to_blob(embedding))
            .bind(Utc::now())
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn rename(pool: &SqlitePool, id: &str, name: &str) -> Result<(), SqlxError> {
        sqlx::query("UPDATE speaker_profiles SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(Utc::now())
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), SqlxError> {
        // Delete exemplars explicitly (don't rely on FK cascade, which needs
        // PRAGMA foreign_keys=ON), then the profile row.
        sqlx::query("DELETE FROM speaker_profile_embeddings WHERE profile_id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        sqlx::query("DELETE FROM speaker_profiles WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    // --- Multi-exemplar enrollment -----------------------------------------

    /// All saved profiles with their stored exemplars (for matching / flagging).
    /// Profiles with no exemplars are omitted (they can't be matched).
    pub async fn list_with_exemplars(
        pool: &SqlitePool,
    ) -> Result<Vec<ProfileExemplars>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, String, Vec<u8>)>(
            "SELECT p.id, p.name, e.embedding \
             FROM speaker_profiles p \
             JOIN speaker_profile_embeddings e ON e.profile_id = p.id \
             ORDER BY p.id, e.created_at",
        )
        .fetch_all(pool)
        .await?;

        let mut out: Vec<ProfileExemplars> = Vec::new();
        for (id, name, blob) in rows {
            let emb = blob_to_embedding(&blob);
            match out.last_mut() {
                Some(p) if p.id == id => p.exemplars.push(emb),
                _ => out.push(ProfileExemplars {
                    id,
                    name,
                    exemplars: vec![emb],
                }),
            }
        }
        Ok(out)
    }

    /// (exemplar id, embedding) rows for one profile, oldest first.
    async fn exemplar_rows_for(
        pool: &SqlitePool,
        profile_id: &str,
    ) -> Result<Vec<(String, Vec<f32>)>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, Vec<u8>)>(
            "SELECT id, embedding FROM speaker_profile_embeddings \
             WHERE profile_id = ? ORDER BY created_at",
        )
        .bind(profile_id)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, b)| (id, blob_to_embedding(&b)))
            .collect())
    }

    /// Add a new voice exemplar to a profile. If this pushes the profile over
    /// `max_exemplars`, the most-redundant exemplar is evicted. The profile's
    /// summary centroid (`speaker_profiles.embedding`) is then recomputed.
    pub async fn add_exemplar(
        pool: &SqlitePool,
        profile_id: &str,
        embedding: &[f32],
        max_exemplars: usize,
    ) -> Result<(), SqlxError> {
        Self::insert_exemplar_row(pool, profile_id, embedding).await?;

        let rows = Self::exemplar_rows_for(pool, profile_id).await?;
        if rows.len() > max_exemplars.max(1) {
            let embs: Vec<Vec<f32>> = rows.iter().map(|(_, e)| e.clone()).collect();
            if let Some(idx) = most_redundant_exemplar_index(&embs) {
                sqlx::query("DELETE FROM speaker_profile_embeddings WHERE id = ?")
                    .bind(&rows[idx].0)
                    .execute(pool)
                    .await?;
            }
        }
        Self::refresh_summary(pool, profile_id).await
    }

    async fn insert_exemplar_row(
        pool: &SqlitePool,
        profile_id: &str,
        embedding: &[f32],
    ) -> Result<(), SqlxError> {
        sqlx::query(
            "INSERT INTO speaker_profile_embeddings (id, profile_id, embedding, created_at) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(format!("spx-{}", Uuid::new_v4()))
        .bind(profile_id)
        .bind(embedding_to_blob(embedding))
        .bind(Utc::now())
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Recompute a profile's summary centroid as the normalized mean of its
    /// current exemplars (no-op if it somehow has none).
    async fn refresh_summary(pool: &SqlitePool, profile_id: &str) -> Result<(), SqlxError> {
        let exemplars: Vec<Vec<f32>> = Self::exemplar_rows_for(pool, profile_id)
            .await?
            .into_iter()
            .map(|(_, e)| e)
            .collect();
        if let Some(summary) = mean_normalized(&exemplars) {
            Self::update_embedding(pool, profile_id, &summary).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_roundtrip() {
        let embedding = vec![0.5f32, -1.25, 3.75, 0.0];
        let blob = embedding_to_blob(&embedding);
        assert_eq!(blob.len(), 16);
        assert_eq!(blob_to_embedding(&blob), embedding);
    }

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    #[test]
    fn mean_normalized_is_unit_and_averages() {
        // Mean of (1,0) and (0,1) is (0.5,0.5) -> normalized (1/√2, 1/√2).
        let m = mean_normalized(&[unit(vec![1.0, 0.0]), unit(vec![0.0, 1.0])]).unwrap();
        let e = 1.0f32 / 2.0f32.sqrt();
        assert!((m[0] - e).abs() < 1e-6 && (m[1] - e).abs() < 1e-6);
        assert_eq!(mean_normalized(&[]), None);
    }

    #[test]
    fn most_redundant_picks_the_near_duplicate() {
        // Two near-identical vectors + one distinct: one of the duplicates is
        // the most redundant (highest nearest-neighbour cosine) and gets evicted.
        let a = unit(vec![1.0, 0.0, 0.0]);
        let a2 = unit(vec![0.98, 0.05, 0.0]);
        let b = unit(vec![0.0, 1.0, 0.0]);
        let idx = most_redundant_exemplar_index(&[a, a2, b]).unwrap();
        assert!(idx == 0 || idx == 1, "expected a duplicate (idx 0/1), got {idx}");
        assert_eq!(most_redundant_exemplar_index(&[unit(vec![1.0, 0.0])]), None);
    }
}

#[cfg(test)]
mod accrual_tests {
    use super::*;
    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }
    #[test]
    fn accrue_moves_toward_new_and_stays_unit() {
        let existing = unit(vec![1.0, 0.0, 0.0]);
        let new = unit(vec![0.0, 1.0, 0.0]);
        let out = accrue_centroid(&existing, 4, &new); // 4 prior segments
        let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "result must be unit-norm, got {norm}");
        // moved toward `new` on axis 1 but still dominated by `existing` on axis 0
        assert!(out[0] > out[1] && out[1] > 0.0);
    }
}

#[cfg(test)]
mod db_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    async fn test_pool() -> SqlitePool {
        // Single connection so the in-memory schema persists across queries.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn exemplar_lifecycle_create_add_cap_delete() {
        let pool = test_pool().await;

        // create() seeds the profile's first exemplar.
        let id = SpeakerProfilesRepository::create(&pool, "Alice", &unit(vec![1.0, 0.0, 0.0]))
            .await
            .unwrap();
        let listed = SpeakerProfilesRepository::list_with_exemplars(&pool)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Alice");
        assert_eq!(listed[0].exemplars.len(), 1);

        // Adding beyond a cap of 2 evicts, so the count stays at 2.
        SpeakerProfilesRepository::add_exemplar(&pool, &id, &unit(vec![0.0, 1.0, 0.0]), 2)
            .await
            .unwrap();
        SpeakerProfilesRepository::add_exemplar(&pool, &id, &unit(vec![0.0, 0.0, 1.0]), 2)
            .await
            .unwrap();
        let listed = SpeakerProfilesRepository::list_with_exemplars(&pool)
            .await
            .unwrap();
        assert_eq!(listed[0].exemplars.len(), 2, "should cap at max_exemplars");

        // The summary centroid is maintained (unit-norm) and delete cascades.
        let summary = SpeakerProfilesRepository::list(&pool).await.unwrap()[0]
            .embedding
            .clone();
        let norm = summary.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "summary must be unit-norm, got {norm}");

        SpeakerProfilesRepository::delete(&pool, &id).await.unwrap();
        assert!(SpeakerProfilesRepository::list_with_exemplars(&pool)
            .await
            .unwrap()
            .is_empty());
    }
}
