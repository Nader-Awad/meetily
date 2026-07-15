// diarization/clustering.rs
//
// Online speaker clustering over L2-normalized embeddings.
// Each incoming embedding joins the nearest cluster centroid if cosine
// similarity exceeds the threshold, otherwise starts a new "Speaker N"
// cluster. Centroids are running means, re-normalized after update.

use super::normalize::{center_normalized, cohort_mean, MIN_PROFILES_FOR_CENTERING};

/// Minimum cosine similarity for an embedding to join an existing cluster.
/// Tuned for WeSpeaker CAM++ embeddings; raise to split more aggressively.
pub const CLUSTER_SIMILARITY_THRESHOLD: f32 = 0.55;

/// Minimum cosine similarity for a new cluster to match a saved voice profile.
/// RAW-space fallback, used only when there are too few saved profiles to
/// estimate the shared anisotropy direction (see `MIN_PROFILES_FOR_CENTERING`).
pub const PROFILE_MATCH_THRESHOLD: f32 = 0.60;

/// Auto-adopt threshold for a saved profile in CENTERED (anisotropy-corrected)
/// space, used live when enough profiles are seeded. CAM++ embeddings are
/// anisotropic — raw cross-speaker cosine is ~0.8, so the raw 0.60 bar above
/// matches nearly everyone; centering + a competitive margin is what actually
/// separates voices. Conservative (favors "Speaker N" over a wrong name);
/// tunable. See `diarization::normalize` and `diarization::batch`.
pub const CENTERED_PROFILE_MATCH_THRESHOLD: f32 = 0.60;
/// Best-vs-runner-up margin (in centered space) a profile must beat the next
/// profile by to be adopted live — the real protection against wrong names.
pub const CENTERED_PROFILE_MATCH_MARGIN: f32 = 0.12;

/// Default speaker cap per meeting. High enough for typical team meetings;
/// prevents runaway cluster creation from noisy embeddings.
pub const DEFAULT_MAX_ANONYMOUS_SPEAKERS: usize = 10;

pub struct SpeakerCluster {
    pub centroid: Vec<f32>,
    pub count: usize,
    pub label: String,
    /// True for clusters seeded from saved voice profiles (not counted
    /// toward "Speaker N" numbering and excluded from centroid snapshots
    /// until they have real segments).
    pub from_profile: bool,
}

pub struct SpeakerClusterer {
    clusters: Vec<SpeakerCluster>,
    last_label: Option<String>,
    anon_speaker_count: usize,
    max_anonymous_speakers: usize,
    /// Estimated shared anisotropy direction (mean of seeded profile centroids),
    /// used to center embeddings before comparing them to saved profiles.
    /// `None` until estimated, and stays `None` if too few profiles were seeded.
    profile_center: Option<Vec<f32>>,
    /// Whether `profile_center` has been estimated yet (memoized on first assign).
    center_ready: bool,
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    // Inputs are L2-normalized, so the dot product is the cosine similarity
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

impl SpeakerClusterer {
    pub fn new() -> Self {
        Self::with_max_anonymous_speakers(DEFAULT_MAX_ANONYMOUS_SPEAKERS)
    }

    pub fn with_max_anonymous_speakers(max_anonymous_speakers: usize) -> Self {
        Self {
            clusters: Vec::new(),
            last_label: None,
            anon_speaker_count: 0,
            max_anonymous_speakers: max_anonymous_speakers.max(1),
            profile_center: None,
            center_ready: false,
        }
    }

    /// Seed the clusterer with a saved voice profile so returning speakers
    /// are recognized by name instead of getting an anonymous label.
    ///
    /// Seed ALL profiles before the first `assign()`: the shared-anisotropy
    /// center is estimated once from the seeded set on the first assign, so a
    /// profile added afterwards would be excluded from that estimate.
    pub fn seed_profile(&mut self, name: &str, centroid: Vec<f32>) {
        debug_assert!(
            !self.center_ready,
            "seed_profile called after the anisotropy center was estimated; seed all profiles before the first assign()"
        );
        self.clusters.push(SpeakerCluster {
            centroid,
            count: 0,
            label: name.to_string(),
            from_profile: true,
        });
    }

    /// Label of the most recently assigned segment (used to carry labels
    /// across segments too short for reliable embeddings).
    pub fn last_label(&self) -> Option<String> {
        self.last_label.clone()
    }

    pub fn anon_speaker_count(&self) -> usize {
        self.anon_speaker_count
    }

    /// Assign an L2-normalized embedding to a cluster, returning its label.
    ///
    /// When enough saved profiles have been seeded to estimate the shared
    /// anisotropy direction, a segment first gets a chance to adopt a saved
    /// voice via an anisotropy-corrected, competitive match (a clear winner in
    /// centered space). If there is no clear profile winner it falls through to
    /// ordinary online clustering and — for a genuinely new voice — an anonymous
    /// "Speaker N" label, rather than being force-named by a weak raw match.
    /// With too few profiles the match uses the raw threshold (prior behavior).
    ///
    /// Known trade-off: the gate is re-evaluated per segment, so a returning
    /// speaker whose FIRST segment is ambiguous may briefly land in a "Speaker N"
    /// cluster before a later, cleaner segment adopts their profile name — one
    /// person can then appear under both labels in a meeting. This is rare
    /// (same-speaker segments are usually consistent) and locally fixable via
    /// rename; we accept it rather than re-introduce a cross-cluster
    /// re-attribution sweep (the source of an earlier oscillation bug).
    pub fn assign(&mut self, embedding: &[f32]) -> String {
        self.ensure_profile_center();

        // Preferred path: anisotropy-corrected, competitive adoption of a saved
        // profile. Only fires when the shared direction could be estimated.
        if self.profile_center.is_some() {
            if let Some(idx) = self.confident_profile_match(embedding) {
                let label = self.update_cluster(idx, embedding);
                self.last_label = Some(label.clone());
                return label;
            }
        }

        // Ordinary online clustering. When centering is active, unmatched seeded
        // profiles are excluded here — they can only be adopted by the gate
        // above — so raw anisotropy can't force a weak profile name. When it is
        // inactive (few profiles), this is exactly the original behavior,
        // including the raw PROFILE_MATCH_THRESHOLD for seeded profiles.
        let exclude_unmatched_profiles = self.profile_center.is_some();
        let best = self
            .clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| !(exclude_unmatched_profiles && c.from_profile && c.count == 0))
            .map(|(i, c)| (i, cosine_similarity(embedding, &c.centroid)))
            .filter(|(i, similarity)| {
                let cluster = &self.clusters[*i];
                let threshold = if cluster.from_profile && cluster.count == 0 {
                    PROFILE_MATCH_THRESHOLD
                } else {
                    CLUSTER_SIMILARITY_THRESHOLD
                };
                *similarity >= threshold
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let label = if let Some((idx, _)) = best {
            self.update_cluster(idx, embedding)
        } else if self.anon_speaker_count >= self.max_anonymous_speakers {
            self.nearest_active_cluster_label(embedding)
                .unwrap_or_else(|| self.create_anonymous_cluster(embedding))
        } else {
            self.create_anonymous_cluster(embedding)
        };

        self.last_label = Some(label.clone());
        label
    }

    /// Lazily (once) estimate the shared anisotropy direction as the mean of the
    /// seeded profile centroids, when there are at least
    /// `MIN_PROFILES_FOR_CENTERING` of them. Memoized via `center_ready`.
    ///
    /// Estimated from the seeded profiles ONLY (not the meeting's live clusters),
    /// deliberately: this keeps the center stable for the whole meeting so it can
    /// be computed once. (The batch `ProfileMatcher` instead folds the meeting's
    /// local centroids into its cohort, since it has them all up front.)
    fn ensure_profile_center(&mut self) {
        if self.center_ready {
            return;
        }
        self.center_ready = true;
        let profile_centroids: Vec<&[f32]> = self
            .clusters
            .iter()
            .filter(|c| c.from_profile)
            .map(|c| c.centroid.as_slice())
            .collect();
        if profile_centroids.len() >= MIN_PROFILES_FOR_CENTERING {
            self.profile_center = cohort_mean(&profile_centroids);
        }
    }

    /// Among the unmatched seeded-profile clusters, return the index of the one
    /// this embedding clearly matches in anisotropy-corrected (centered) space:
    /// centered cosine >= `CENTERED_PROFILE_MATCH_THRESHOLD` AND ahead of the
    /// runner-up profile by >= `CENTERED_PROFILE_MATCH_MARGIN`. `None` if there
    /// is no clear winner (the segment then clusters anonymously). Requires
    /// `profile_center`; the margin (not the absolute threshold) is the real
    /// protection against wrong names in this dense, anisotropic space.
    fn confident_profile_match(&self, embedding: &[f32]) -> Option<usize> {
        let center = self.profile_center.as_ref()?;
        let q = center_normalized(embedding, center);
        let mut scored: Vec<(usize, f32)> = self
            .clusters
            .iter()
            .enumerate()
            .filter(|(_, c)| c.from_profile && c.count == 0)
            .map(|(i, c)| {
                let pc = center_normalized(&c.centroid, center);
                (i, cosine_similarity(&q, &pc))
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let (best_idx, best) = *scored.first()?;
        // If only one unmatched profile remains, runner_up defaults to 0.0
        // (~the centered impostor median), so the gate reduces to the absolute
        // `best >= THRESHOLD` bar — a strong bar in centered space. Mirrors the
        // batch path's single-profile handling; don't "simplify" to -1.0/-inf.
        let runner_up = scored.get(1).map(|(_, s)| *s).unwrap_or(0.0);
        if best >= CENTERED_PROFILE_MATCH_THRESHOLD
            && (best - runner_up) >= CENTERED_PROFILE_MATCH_MARGIN
        {
            Some(best_idx)
        } else {
            None
        }
    }

    fn update_cluster(&mut self, idx: usize, embedding: &[f32]) -> String {
        let cluster = &mut self.clusters[idx];
        let n = cluster.count as f32;
        for (c, e) in cluster.centroid.iter_mut().zip(embedding.iter()) {
            *c = (*c * n + e) / (n + 1.0);
        }
        let norm = cluster.centroid.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for c in &mut cluster.centroid {
                *c /= norm;
            }
        }
        cluster.count += 1;
        cluster.label.clone()
    }

    fn create_anonymous_cluster(&mut self, embedding: &[f32]) -> String {
        self.anon_speaker_count += 1;
        let label = format!("Speaker {}", self.anon_speaker_count);
        self.clusters.push(SpeakerCluster {
            centroid: embedding.to_vec(),
            count: 1,
            label: label.clone(),
            from_profile: false,
        });
        label
    }

    fn nearest_active_cluster_label(&self, embedding: &[f32]) -> Option<String> {
        self.clusters
            .iter()
            .filter(|cluster| !cluster.from_profile || cluster.count > 0)
            .map(|cluster| (cluster, cosine_similarity(embedding, &cluster.centroid)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(cluster, _)| cluster.label.clone())
    }

    /// Snapshot of (label, centroid, segment count) for profile persistence.
    /// Profile-seeded clusters that never matched audio are excluded.
    pub fn centroids(&self) -> impl Iterator<Item = (&str, &[f32], usize)> {
        self.clusters
            .iter()
            .filter(|c| c.count > 0)
            .map(|c| (c.label.as_str(), c.centroid.as_slice(), c.count))
    }

    /// Replace a cluster's label (e.g. when matched to a saved voice profile).
    pub fn relabel(&mut self, old_label: &str, new_label: &str) {
        for cluster in &mut self.clusters {
            if cluster.label == old_label {
                cluster.label = new_label.to_string();
            }
        }
        if self.last_label.as_deref() == Some(old_label) {
            self.last_label = Some(new_label.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    #[test]
    fn same_voice_same_cluster() {
        let mut c = SpeakerClusterer::new();
        let a = unit(vec![1.0, 0.1, 0.0]);
        let a2 = unit(vec![0.95, 0.15, 0.05]);
        assert_eq!(c.assign(&a), "Speaker 1");
        assert_eq!(c.assign(&a2), "Speaker 1");
    }

    #[test]
    fn different_voice_new_cluster() {
        let mut c = SpeakerClusterer::new();
        let a = unit(vec![1.0, 0.0, 0.0]);
        let b = unit(vec![0.0, 1.0, 0.0]);
        assert_eq!(c.assign(&a), "Speaker 1");
        assert_eq!(c.assign(&b), "Speaker 2");
        assert_eq!(c.last_label().as_deref(), Some("Speaker 2"));
    }

    #[test]
    fn caps_anonymous_speakers_and_reuses_existing_label_for_outliers() {
        let mut c = SpeakerClusterer::with_max_anonymous_speakers(2);
        let a = unit(vec![1.0, 0.0, 0.0]);
        let b = unit(vec![0.0, 1.0, 0.0]);
        let outlier = unit(vec![0.0, 0.0, 1.0]);

        assert_eq!(c.assign(&a), "Speaker 1");
        assert_eq!(c.assign(&b), "Speaker 2");

        let assigned = c.assign(&outlier);

        assert!(matches!(assigned.as_str(), "Speaker 1" | "Speaker 2"));
        assert_eq!(c.centroids().count(), 2);
        assert_eq!(c.anon_speaker_count(), 2);
    }

    // --- anisotropy-corrected live profile matching ------------------------
    //
    // With enough seeded profiles, live profile adoption is centered + gated by
    // a competitive margin. The prior tests seed zero profiles, so they pin the
    // raw-fallback path (profile_center stays None -> original behavior).

    /// 9-dim anisotropic embedding: a large shared component on every axis plus
    /// a bump on one speaker axis (raw cross-speaker cosine ~0.98).
    fn aniso9(axis: usize, bump: f32) -> Vec<f32> {
        let mut v = vec![3.0f32; 9];
        v[axis] += 1.0 + bump;
        unit(v)
    }

    /// Seed 8 profiles on axes 0..8 (>= MIN_PROFILES_FOR_CENTERING), leaving
    /// axis 8 free for a "new voice" not represented by any profile.
    fn seed_eight(c: &mut SpeakerClusterer) {
        for i in 0..8 {
            c.seed_profile(&format!("P{i}"), aniso9(i, 0.0));
        }
    }

    #[test]
    fn live_adopts_correct_saved_profile() {
        let mut c = SpeakerClusterer::new();
        seed_eight(&mut c);
        // A segment clearly from P3 is adopted by name (centered clear winner).
        assert_eq!(c.assign(&aniso9(3, 0.5)), "P3");
    }

    #[test]
    fn live_new_voice_stays_anonymous_under_centering() {
        let mut c = SpeakerClusterer::new();
        seed_eight(&mut c);
        // A distinct 9th voice: raw cosine to every profile clears the raw 0.60
        // bar (shared component dominates), so the OLD raw path would force-name
        // it. Centered, it matches no profile clearly and must stay "Speaker N".
        let newcomer = aniso9(8, 0.5);
        let raw = cosine_similarity(&newcomer, &aniso9(0, 0.0));
        assert!(raw > PROFILE_MATCH_THRESHOLD, "raw {raw} should clear the raw bar");
        assert_eq!(c.assign(&newcomer), "Speaker 1");
    }

    #[test]
    fn live_few_profiles_use_raw_fallback() {
        // Fewer than MIN_PROFILES_FOR_CENTERING profiles -> no centering; a close
        // segment still adopts a profile via the raw path (previous behavior).
        let mut c = SpeakerClusterer::new();
        c.seed_profile("P0", unit(vec![1.0, 0.0, 0.0]));
        c.seed_profile("P1", unit(vec![0.0, 1.0, 0.0]));
        assert_eq!(c.assign(&unit(vec![0.98, 0.05, 0.0])), "P0");
    }

    #[test]
    fn live_matched_profile_keeps_name_on_next_segment() {
        let mut c = SpeakerClusterer::new();
        seed_eight(&mut c);
        assert_eq!(c.assign(&aniso9(3, 0.5)), "P3");
        // A second, slightly different segment of the same speaker stays P3
        // (now an active cluster matched via ordinary clustering).
        assert_eq!(c.assign(&aniso9(3, 0.35)), "P3");
    }
}
