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
        sqlx::query("DELETE FROM speaker_profiles WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
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
