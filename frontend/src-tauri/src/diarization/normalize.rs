// diarization/normalize.rs
//
// Anisotropy correction for speaker embeddings.
//
// WeSpeaker CAM++ embeddings are strongly *anisotropic*: every embedding shares
// a large common direction (loosely, the "mean voice" of the model's training
// distribution). Measured on real saved profiles, each unit-length embedding
// sits at cosine ~0.91 to the mean of all embeddings, so raw cross-speaker
// cosine lands around 0.8 and DIFFERENT people look nearly identical — the
// speaker-specific signal is swamped by the shared component. Worse, averaging
// many segments into a centroid (enrollment/accrual) reinforces the shared part
// and washes out the speaker residual, so more data makes matching *worse*.
//
// The standard remedy for embedding-based speaker ID is score/embedding
// normalization: estimate the shared direction from a cohort of embeddings,
// subtract it, and re-normalize. The residual then reflects speaker identity,
// and cross-speaker cosine collapses toward 0 while same-speaker stays high —
// restoring a usable decision margin.
//
// This module is pure (no I/O, no model calls) and fully unit-testable.

/// Minimum number of *saved profiles* required before we trust a mean estimate
/// of the shared anisotropy direction and switch on centering. Gating on the
/// profile count (which is stable for a given user) rather than a per-call
/// cohort size keeps the scoring regime consistent from meeting to meeting.
/// Below this, mean estimation is too noisy / ill-conditioned (e.g. two vectors
/// centered by their own mean become antipodal), so callers stay in raw space.
pub const MIN_PROFILES_FOR_CENTERING: usize = 8;

/// Element-wise mean of a cohort of equal-length embeddings.
///
/// Embeddings whose length differs from the first usable one are skipped
/// (defensive guard against a mismatched-dimension vector). Returns `None` if
/// the cohort is empty or contains no usable embedding.
///
/// The returned mean is intentionally **not** re-normalized: its magnitude is
/// itself diagnostic — a norm near 1.0 means the cohort is highly anisotropic
/// (a strong shared direction), while a norm near 0.0 means the embeddings are
/// already well spread out and centering will do little.
pub fn cohort_mean(embeddings: &[&[f32]]) -> Option<Vec<f32>> {
    let dim = embeddings.iter().find(|e| !e.is_empty())?.len();
    let mut sum = vec![0.0f32; dim];
    let mut count = 0usize;
    for emb in embeddings {
        if emb.len() == dim {
            for (acc, v) in sum.iter_mut().zip(emb.iter()) {
                *acc += v;
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
    Some(sum)
}

/// Subtract the cohort `mean` from `vec` and re-L2-normalize, removing the
/// shared anisotropy direction so the residual reflects speaker identity.
///
/// Falls back to returning `vec` unchanged (as-is) when there is nothing to
/// correct: mismatched dimensions, an empty input, or a residual whose norm is
/// ~0 (i.e. the embedding equals the mean). Callers pass already-L2-normalized
/// embeddings, so the unchanged fallback is still unit-length.
pub fn center_normalized(vec: &[f32], mean: &[f32]) -> Vec<f32> {
    if vec.is_empty() || vec.len() != mean.len() {
        return vec.to_vec();
    }
    let mut out: Vec<f32> = vec.iter().zip(mean).map(|(v, m)| v - m).collect();
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm <= 1e-6 {
        return vec.to_vec();
    }
    for x in &mut out {
        *x /= norm;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::clustering::cosine_similarity;

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    /// Build a realistic anisotropic embedding: a large shared component plus a
    /// small speaker-specific part. Mirrors real CAM++ behavior where distinct
    /// speakers still sit at ~0.8 raw cosine.
    fn aniso(speaker: [f32; 4]) -> Vec<f32> {
        let shared = [4.0f32, 4.0, 4.0, 4.0];
        unit(vec![
            shared[0] + speaker[0],
            shared[1] + speaker[1],
            shared[2] + speaker[2],
            shared[3] + speaker[3],
        ])
    }

    #[test]
    fn cohort_mean_empty_is_none() {
        let empty: Vec<&[f32]> = Vec::new();
        assert_eq!(cohort_mean(&empty), None);
    }

    #[test]
    fn cohort_mean_skips_mismatched_dims() {
        let a = [1.0f32, 0.0];
        let bad = [1.0f32, 0.0, 0.0];
        let b = [0.0f32, 1.0];
        let mean = cohort_mean(&[&a[..], &bad[..], &b[..]]).unwrap();
        // bad (len 3) skipped; mean of (1,0) and (0,1) is (0.5, 0.5)
        assert_eq!(mean.len(), 2);
        assert!((mean[0] - 0.5).abs() < 1e-6 && (mean[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn center_normalized_returns_unit_vector() {
        let v = aniso([1.0, 0.0, 0.0, 0.0]);
        let mean = [0.5f32, 0.5, 0.5, 0.5];
        let c = center_normalized(&v, &mean);
        let norm = c.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "expected unit vector, got norm {norm}");
    }

    #[test]
    fn center_normalized_degenerate_returns_input() {
        // vec == mean -> zero residual -> return input unchanged.
        let v = unit(vec![1.0, 1.0, 1.0, 1.0]);
        let c = center_normalized(&v, &v);
        assert_eq!(c, v);
    }

    #[test]
    fn center_normalized_dim_mismatch_returns_input() {
        let v = vec![1.0f32, 0.0];
        let mean = [0.5f32, 0.5, 0.5];
        assert_eq!(center_normalized(&v, &mean), v);
    }

    #[test]
    fn centering_restores_speaker_separation() {
        // Three distinct speakers, all anisotropic. Raw cosine puts them all
        // close together (the shared component dominates); centering by the
        // cohort mean pulls them apart.
        let a = aniso([1.0, 0.0, 0.0, 0.0]);
        let b = aniso([0.0, 1.0, 0.0, 0.0]);
        let c = aniso([0.0, 0.0, 1.0, 0.0]);

        let raw_ab = cosine_similarity(&a, &b);
        assert!(raw_ab > 0.9, "fixture should be anisotropic: raw cos {raw_ab}");

        let mean = cohort_mean(&[&a[..], &b[..], &c[..]]).unwrap();
        let ca = center_normalized(&a, &mean);
        let cb = center_normalized(&b, &mean);
        let cen_ab = cosine_similarity(&ca, &cb);

        assert!(
            cen_ab < raw_ab - 0.3,
            "centering should sharply reduce cross-speaker cosine: raw {raw_ab} -> centered {cen_ab}"
        );
    }
}
