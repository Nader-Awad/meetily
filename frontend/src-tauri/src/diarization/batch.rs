// diarization/batch.rs
//
// Pure turn→unit merging and speakrs-speaker→saved-profile mapping for the
// batch (Retranscribe/Import) diarization path. No I/O, no model calls here —
// callers supply CAM++ centroids; this module is fully unit-testable.

use super::clustering::cosine_similarity;
use super::normalize::{center_normalized, cohort_mean};
use super::segmenter::DiarTurn;
use std::collections::HashMap;

pub const MIN_UNIT_MS: u64 = 1000;
pub const MERGE_GAP_MS: u64 = 500;

// --- Profile-matching thresholds -------------------------------------------
//
// A cluster only adopts a saved profile's name when it is a CLEAR winner:
// nearest profile, cosine ≥ the high-match threshold, and ahead of the
// runner-up by ≥ the match margin. Otherwise the cluster stays "Speaker N"
// (unknown) — we never force a guess.
//
// There are TWO threshold regimes because we score in one of two spaces
// depending on how much enrolled data is available (see `ProfileMatcher`):
//
//  * CENTERED space (preferred): CAM++ embeddings are strongly anisotropic
//    (every embedding shares a large common direction), so raw cross-speaker
//    cosine sits ~0.8 and the raw thresholds below are unusable — the mean
//    impostor score (~0.78) is *above* the old 0.72 auto-adopt bar. When the
//    cohort (this meeting's clusters ∪ saved profiles) is large enough to
//    estimate the shared direction, we subtract it (see `normalize`) and score
//    on the residual, where cross-speaker cosine collapses toward 0. The
//    CENTERED_* thresholds are calibrated for that space (impostor median ~0.1,
//    same-speaker median ~0.7). Values are conservative — tune toward higher
//    recall (lower threshold) or higher precision (higher threshold) as needed.
//
//  * RAW space (fallback): when too few embeddings exist to estimate the shared
//    direction (e.g. a brand-new user with one or two profiles), centering is
//    ill-conditioned, so we fall back to raw cosine with the original,
//    deliberately strict thresholds.

/// Auto-adopt threshold in RAW cosine space (fallback regime).
pub const HIGH_MATCH_THRESHOLD: f32 = 0.72;
/// Best-vs-runner-up margin in RAW cosine space (fallback regime).
pub const MATCH_MARGIN: f32 = 0.08;

/// Auto-adopt threshold in CENTERED (anisotropy-corrected) space.
pub const CENTERED_HIGH_MATCH_THRESHOLD: f32 = 0.60;
/// Best-vs-runner-up margin in CENTERED space (residual margins are larger).
pub const CENTERED_MATCH_MARGIN: f32 = 0.12;
/// Near-match suggestion floor in CENTERED space (analogue of `SUGGEST_FLOOR`).
pub const CENTERED_SUGGEST_FLOOR: f32 = 0.42;

/// Minimum cohort size (local clusters + saved profiles) required before we
/// trust a cohort-mean estimate of the anisotropy direction and switch to
/// centered scoring. Below this, mean-centering is degenerate (e.g. two vectors
/// centered by their own mean become antipodal), so we stay in raw space.
pub const MIN_COHORT_FOR_CENTERING: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct DiarUnit {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker_local: String,
}

/// Merge adjacent same-speaker turns (gap ≤ merge_gap_ms) and drop units
/// shorter than min_unit_ms. Turns are assumed time-ordered.
pub fn merge_turns(turns: &[DiarTurn], min_unit_ms: u64, merge_gap_ms: u64) -> Vec<DiarUnit> {
    let mut units: Vec<DiarUnit> = Vec::new();
    for turn in turns {
        if let Some(last) = units.last_mut() {
            if last.speaker_local == turn.speaker
                && turn.start_ms >= last.end_ms
                && turn.start_ms - last.end_ms <= merge_gap_ms
            {
                last.end_ms = turn.end_ms.max(last.end_ms);
                continue;
            }
        }
        units.push(DiarUnit {
            start_ms: turn.start_ms,
            end_ms: turn.end_ms,
            speaker_local: turn.speaker.clone(),
        });
    }
    units
        .into_iter()
        .filter(|u| u.end_ms.saturating_sub(u.start_ms) >= min_unit_ms)
        .collect()
}

/// Scores clusters against saved profiles with anisotropy correction.
///
/// On construction it picks a scoring regime from how much enrolled data is
/// available. If the cohort (this meeting's local cluster centroids ∪ the saved
/// profile embeddings) is at least `MIN_COHORT_FOR_CENTERING`, it estimates the
/// shared anisotropy direction (`cohort_mean`), pre-centers the profile
/// embeddings, and uses the CENTERED_* thresholds; otherwise it keeps the raw
/// embeddings and the strict RAW thresholds. Callers rank a query cluster with
/// `ranked` and read `high`/`margin`/`suggest_floor` for the gate. Pure.
struct ProfileMatcher<'a> {
    /// (name, embedding) in the active scoring space (centered when `mean` set).
    profiles: Vec<(&'a str, Vec<f32>)>,
    /// `Some(mean)` in the centered regime — queries get centered by it too.
    mean: Option<Vec<f32>>,
    high: f32,
    margin: f32,
    suggest_floor: f32,
}

impl<'a> ProfileMatcher<'a> {
    fn new(local_centroids: &[(String, Vec<f32>)], profiles: &'a [(String, Vec<f32>)]) -> Self {
        let cohort: Vec<&[f32]> = local_centroids
            .iter()
            .map(|(_, c)| c.as_slice())
            .chain(profiles.iter().map(|(_, e)| e.as_slice()))
            .collect();

        // Only trust a cohort-mean estimate of the shared direction once we have
        // enough embeddings; below that, centering is ill-conditioned.
        let mean = if cohort.len() >= MIN_COHORT_FOR_CENTERING {
            cohort_mean(&cohort)
        } else {
            None
        };

        let scored_profiles = profiles
            .iter()
            .map(|(name, emb)| {
                let v = match &mean {
                    Some(m) => center_normalized(emb, m),
                    None => emb.clone(),
                };
                (name.as_str(), v)
            })
            .collect();

        let (high, margin, suggest_floor) = if mean.is_some() {
            (
                CENTERED_HIGH_MATCH_THRESHOLD,
                CENTERED_MATCH_MARGIN,
                CENTERED_SUGGEST_FLOOR,
            )
        } else {
            (HIGH_MATCH_THRESHOLD, MATCH_MARGIN, SUGGEST_FLOOR)
        };

        Self {
            profiles: scored_profiles,
            mean,
            high,
            margin,
            suggest_floor,
        }
    }

    /// True when anisotropy correction is active (cohort large enough).
    #[cfg(test)]
    fn is_centered(&self) -> bool {
        self.mean.is_some()
    }

    /// Profiles ranked best-first by similarity to `query`, in the active
    /// scoring space. `query` is centered by the cohort mean when centering is
    /// active. Empty when there are no profiles.
    fn ranked(&self, query: &[f32]) -> Vec<(&'a str, f32)> {
        let q = match &self.mean {
            Some(m) => center_normalized(query, m),
            None => query.to_vec(),
        };
        let mut sims: Vec<(&'a str, f32)> = self
            .profiles
            .iter()
            .map(|(name, emb)| (*name, cosine_similarity(&q, emb)))
            .collect();
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        sims
    }
}

/// Map each local speakrs speaker to a saved profile name, but only when the
/// match is a clear, confident win. Scoring is anisotropy-corrected when enough
/// enrolled data exists (see `ProfileMatcher`); the confident-winner gate is
/// then applied with the regime's thresholds. Otherwise falls back to a stable
/// "Speaker N" label (numbered by first appearance) — we never force a guess.
pub fn map_local_speakers_to_profiles(
    local_centroids: &[(String, Vec<f32>)],
    profiles: &[(String, Vec<f32>)],
) -> HashMap<String, String> {
    let matcher = ProfileMatcher::new(local_centroids, profiles);
    let mut map = HashMap::new();
    let mut anon: usize = 0;
    for (local, centroid) in local_centroids {
        let sims = matcher.ranked(centroid);

        let confident = match sims.first() {
            Some((name, best)) => {
                let runner_up = sims.get(1).map(|(_, s)| *s).unwrap_or(0.0);
                if *best >= matcher.high && (*best - runner_up) >= matcher.margin {
                    Some((*name).to_string())
                } else {
                    None
                }
            }
            None => None,
        };

        let label = match confident {
            Some(name) => name,
            None => {
                anon += 1;
                format!("Speaker {}", anon)
            }
        };
        map.insert(local.clone(), label);
    }
    map
}

/// Element-wise mean of equal-length CAM++ embeddings, then L2-normalized —
/// the centroid the batch (Retranscribe/Import) diarization path persists per
/// speaker. Mirrors `SpeakerClusterer`'s running-mean update but takes the raw
/// embedding list instead of a running sum, so callers don't need to track a
/// sum/count accumulator by hand.
///
/// Embeddings whose length doesn't match the first embedding's are dropped
/// from the mean (defensive guard against a mismatched-dimension embedding
/// slipping in). Returns `None` if `embeddings` is empty or the mean vector
/// has zero norm (never observed with real CAM++ output, but guarded so
/// callers don't divide by zero).
pub fn average_normalized_centroid(embeddings: &[Vec<f32>]) -> Option<Vec<f32>> {
    let dim = embeddings.first()?.len();
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
    let mut avg: Vec<f32> = sum.iter().map(|v| v / count as f32).collect();
    let norm = avg.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm <= 0.0 {
        return None;
    }
    for v in avg.iter_mut() {
        *v /= norm;
    }
    Some(avg)
}

/// Re-key per-local-speaker embedding lists under their FINAL (profile-mapped)
/// label, combining local speakers that `name_map` maps to the same final
/// name into one pooled centroid + segment count. Local speakers absent from
/// `name_map` are dropped. Used to persist `speakers.json` under the names
/// actually shown in the transcript rather than the diarizer's local labels.
pub fn merge_centroids_by_final_label(
    local_embeddings: &HashMap<String, Vec<Vec<f32>>>,
    name_map: &HashMap<String, String>,
) -> Vec<(String, Vec<f32>, usize)> {
    let mut merged: HashMap<String, Vec<Vec<f32>>> = HashMap::new();
    for (local, embeddings) in local_embeddings {
        if let Some(final_name) = name_map.get(local) {
            merged
                .entry(final_name.clone())
                .or_default()
                .extend(embeddings.iter().cloned());
        }
    }
    merged
        .into_iter()
        .filter_map(|(label, embeddings)| {
            let count = embeddings.len();
            average_normalized_centroid(&embeddings).map(|centroid| (label, centroid, count))
        })
        .collect()
}

/// Near-match suggestion band floor. A cluster that did NOT confidently match
/// (below HIGH_MATCH_THRESHOLD) but whose best profile cosine is at least this,
/// and clearly ahead of the runner-up, is surfaced as a confirmable suggestion
/// rather than a silent "Speaker N". Never auto-applied.
pub const SUGGEST_FLOOR: f32 = 0.62;

/// For each cluster whose final label is still an unnamed "Speaker N" (i.e. it
/// did not confidently match a profile in `name_map`), return the best
/// near-match suggestion (SUGGEST_FLOOR ≤ cosine < HIGH_MATCH_THRESHOLD, clear
/// top candidate by ≥ MATCH_MARGIN), keyed by the cluster's FINAL label. Pure.
pub fn suggest_near_matches(
    local_centroids: &[(String, Vec<f32>)],
    profiles: &[(String, Vec<f32>)],
    name_map: &HashMap<String, String>,
) -> HashMap<String, (String, f32)> {
    let matcher = ProfileMatcher::new(local_centroids, profiles);
    let mut out = HashMap::new();
    for (local, centroid) in local_centroids {
        let final_label = match name_map.get(local) {
            Some(l) => l,
            None => continue,
        };
        // Skip clusters that confidently matched a profile (their final label IS
        // a profile name); only unmatched "Speaker N" clusters get a suggestion.
        if profiles.iter().any(|(n, _)| n == final_label) {
            continue;
        }
        let sims = matcher.ranked(centroid);
        if let Some((name, best)) = sims.first() {
            let runner_up = sims.get(1).map(|(_, s)| *s).unwrap_or(0.0);
            if *best >= matcher.suggest_floor
                && *best < matcher.high
                && (*best - runner_up) >= matcher.margin
            {
                out.insert(final_label.clone(), ((*name).to_string(), *best));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diarization::segmenter::DiarTurn;

    fn t(start: u64, end: u64, spk: &str) -> DiarTurn {
        DiarTurn { start_ms: start, end_ms: end, speaker: spk.to_string() }
    }

    #[test]
    fn merges_adjacent_same_speaker() {
        let turns = vec![t(0, 1200, "A"), t(1400, 2600, "A"), t(2600, 4000, "B")];
        let units = merge_turns(&turns, MIN_UNIT_MS, MERGE_GAP_MS);
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].speaker_local, "A");
        assert_eq!(units[0].start_ms, 0);
        assert_eq!(units[0].end_ms, 2600);
        assert_eq!(units[1].speaker_local, "B");
    }

    #[test]
    fn drops_sub_minimum_units() {
        let turns = vec![t(0, 1500, "A"), t(1500, 1900, "B")]; // B is 400ms < 1000ms
        let units = merge_turns(&turns, MIN_UNIT_MS, MERGE_GAP_MS);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].speaker_local, "A");
    }

    #[test]
    fn does_not_merge_across_large_gap() {
        let turns = vec![t(0, 1200, "A"), t(3000, 4200, "A")]; // 1800ms gap > 500ms
        let units = merge_turns(&turns, MIN_UNIT_MS, MERGE_GAP_MS);
        assert_eq!(units.len(), 2);
    }

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    #[test]
    fn maps_matching_local_speaker_to_profile_else_speaker_n() {
        let alice = unit(vec![1.0, 0.05, 0.0]);
        let locals = vec![
            ("A".to_string(), unit(vec![0.98, 0.1, 0.05])), // close to alice
            ("B".to_string(), unit(vec![0.0, 1.0, 0.0])),   // not alice
        ];
        let profiles = vec![("Alice".to_string(), alice)];
        let map = map_local_speakers_to_profiles(&locals, &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Alice"));
        assert_eq!(map.get("B").map(String::as_str), Some("Speaker 1"));
    }

    #[test]
    fn ambiguous_match_is_unknown_speaker_n() {
        // two profiles both close to the cluster, within the margin → not confident
        let cluster = unit(vec![1.0, 1.0, 0.0]);
        let profiles = vec![
            ("Alice".to_string(), unit(vec![1.0, 0.9, 0.0])),
            ("Bob".to_string(), unit(vec![0.9, 1.0, 0.0])),
        ];
        let map = map_local_speakers_to_profiles(&[("A".to_string(), cluster)], &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Speaker 1"));
    }

    #[test]
    fn weak_best_below_high_threshold_is_unknown() {
        // best match is ~0.64 cosine — above the OLD 0.60 bar (would have mislabeled),
        // below HIGH_MATCH_THRESHOLD → must stay "Speaker 1", not "Alice".
        let cluster = unit(vec![1.0, 1.2, 0.0]);
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let sim = crate::diarization::clustering::cosine_similarity(
            &unit(vec![1.0, 1.2, 0.0]),
            &unit(vec![1.0, 0.0, 0.0]),
        );
        assert!(sim > 0.60 && sim < HIGH_MATCH_THRESHOLD, "test fixture sim={sim}");
        let map = map_local_speakers_to_profiles(&[("A".to_string(), cluster)], &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Speaker 1"));
    }

    #[test]
    fn single_strong_profile_matches() {
        let cluster = unit(vec![1.0, 0.02, 0.0]);
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let map = map_local_speakers_to_profiles(&[("A".to_string(), cluster)], &profiles);
        assert_eq!(map.get("A").map(String::as_str), Some("Alice"));
    }

    #[test]
    fn average_normalized_centroid_empty_is_none() {
        assert_eq!(average_normalized_centroid(&[]), None);
    }

    #[test]
    fn average_normalized_centroid_single_vector_is_normalized() {
        let centroid = average_normalized_centroid(&[vec![3.0, 4.0]]).unwrap();
        assert!((centroid[0] - 0.6).abs() < 1e-6);
        assert!((centroid[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn average_normalized_centroid_two_vectors_mean_then_normalize() {
        // Mean of (2,0) and (0,2) is (1,1); normalized is (1/sqrt2, 1/sqrt2).
        let centroid = average_normalized_centroid(&[vec![2.0, 0.0], vec![0.0, 2.0]]).unwrap();
        let expected = 1.0f32 / 2.0f32.sqrt();
        assert!((centroid[0] - expected).abs() < 1e-6);
        assert!((centroid[1] - expected).abs() < 1e-6);
    }

    #[test]
    fn average_normalized_centroid_zero_norm_is_none() {
        // Mean of (1,0) and (-1,0) is (0,0) — zero norm, can't normalize.
        let centroid = average_normalized_centroid(&[vec![1.0, 0.0], vec![-1.0, 0.0]]);
        assert_eq!(centroid, None);
    }

    #[test]
    fn merge_centroids_by_final_label_combines_matching_locals() {
        let mut local_embeddings: HashMap<String, Vec<Vec<f32>>> = HashMap::new();
        local_embeddings.insert("A".to_string(), vec![vec![1.0, 0.0]]);
        local_embeddings.insert("B".to_string(), vec![vec![1.0, 0.0]]);
        local_embeddings.insert("C".to_string(), vec![vec![0.0, 1.0]]);

        let mut name_map: HashMap<String, String> = HashMap::new();
        name_map.insert("A".to_string(), "Alice".to_string());
        name_map.insert("B".to_string(), "Alice".to_string()); // maps to same final name as A
        name_map.insert("C".to_string(), "Bob".to_string());

        let mut merged = merge_centroids_by_final_label(&local_embeddings, &name_map);
        merged.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].0, "Alice");
        assert_eq!(merged[0].2, 2); // A + B combined
        assert!((merged[0].1[0] - 1.0).abs() < 1e-6);
        assert_eq!(merged[1].0, "Bob");
        assert_eq!(merged[1].2, 1);
    }

    #[test]
    fn merge_centroids_by_final_label_drops_locals_missing_from_name_map() {
        let mut local_embeddings: HashMap<String, Vec<Vec<f32>>> = HashMap::new();
        local_embeddings.insert("A".to_string(), vec![vec![1.0, 0.0]]);
        local_embeddings.insert("Unmapped".to_string(), vec![vec![0.0, 1.0]]);

        let mut name_map: HashMap<String, String> = HashMap::new();
        name_map.insert("A".to_string(), "Alice".to_string());

        let merged = merge_centroids_by_final_label(&local_embeddings, &name_map);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].0, "Alice");
    }

    #[test]
    fn suggests_near_match_for_unmatched_cluster() {
        // best ~0.66 (in [0.62, 0.72)), clear top → suggestion.
        let cluster = unit(vec![1.0, 1.1, 0.0]); // cos to [1,0,0] ~ 0.67
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Speaker 1".to_string()); // unmatched
        let out = suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map);
        let s = out.get("Speaker 1").expect("should suggest");
        assert_eq!(s.0, "Alice");
        assert!(s.1 >= SUGGEST_FLOOR && s.1 < HIGH_MATCH_THRESHOLD, "score {}", s.1);
    }

    #[test]
    fn no_suggestion_for_confidently_matched_cluster() {
        // name_map already resolved to the profile name → not a "Speaker N" → skip.
        let cluster = unit(vec![1.0, 0.02, 0.0]);
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Alice".to_string());
        let out = suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map);
        assert!(out.is_empty());
    }

    #[test]
    fn no_suggestion_below_floor() {
        let cluster = unit(vec![1.0, 2.0, 0.0]); // cos to [1,0,0] ~ 0.447 < 0.62
        let profiles = vec![("Alice".to_string(), unit(vec![1.0, 0.0, 0.0]))];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Speaker 1".to_string());
        assert!(suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map).is_empty());
    }

    #[test]
    fn no_suggestion_when_ambiguous() {
        let cluster = unit(vec![1.0, 1.0, 0.0]);
        let profiles = vec![
            ("Alice".to_string(), unit(vec![1.0, 0.9, 0.0])),
            ("Bob".to_string(), unit(vec![0.9, 1.0, 0.0])),
        ];
        let mut name_map = std::collections::HashMap::new();
        name_map.insert("A".to_string(), "Speaker 1".to_string());
        assert!(suggest_near_matches(&[("A".to_string(), cluster)], &profiles, &name_map).is_empty());
    }

    // --- centered (anisotropy-corrected) regime -----------------------------
    //
    // The small-fixture tests above all have cohorts < MIN_COHORT_FOR_CENTERING,
    // so they exercise (and pin) the RAW fallback path. The tests below build a
    // cohort large enough to trigger centering and verify it separates speakers
    // that raw cosine cannot.

    /// 8-dim anisotropic embedding: a large shared component on every axis plus
    /// a bump on one speaker axis. Distinct speakers still sit at ~0.99 raw
    /// cosine (the shared part dominates), so the raw gate cannot separate them.
    fn aniso8(speaker_axis: usize, bump: f32) -> Vec<f32> {
        let mut v = vec![3.0f32; 8];
        v[speaker_axis] += 1.0 + bump;
        unit(v)
    }

    fn profiles8() -> Vec<(String, Vec<f32>)> {
        (0..8).map(|i| (format!("P{i}"), aniso8(i, 0.0))).collect()
    }

    #[test]
    fn centered_regime_activates_with_enough_cohort() {
        // 2 locals + 8 profiles = 10 >= MIN_COHORT_FOR_CENTERING.
        let locals = vec![
            ("L2".to_string(), aniso8(2, 0.4)),
            ("L5".to_string(), aniso8(5, 0.4)),
        ];
        let profiles = profiles8();
        let matcher = ProfileMatcher::new(&locals, &profiles);
        assert!(matcher.is_centered());
        assert_eq!(matcher.high, CENTERED_HIGH_MATCH_THRESHOLD);
    }

    #[test]
    fn raw_regime_when_cohort_too_small() {
        // 1 local + 3 profiles = 4 < MIN_COHORT_FOR_CENTERING -> raw fallback.
        let locals = vec![("L0".to_string(), aniso8(0, 0.4))];
        let profiles: Vec<(String, Vec<f32>)> =
            (0..3).map(|i| (format!("P{i}"), aniso8(i, 0.0))).collect();
        let matcher = ProfileMatcher::new(&locals, &profiles);
        assert!(!matcher.is_centered());
        assert_eq!(matcher.high, HIGH_MATCH_THRESHOLD);
    }

    #[test]
    fn centered_regime_matches_correct_speaker_where_raw_cannot() {
        let profiles = profiles8();
        let locals = vec![
            ("L2".to_string(), aniso8(2, 0.5)),
            ("L6".to_string(), aniso8(6, 0.5)),
        ];

        // Raw cosine can't tell them apart: best vs runner-up is within the raw
        // margin, so the raw gate would abstain.
        let raw_best = cosine_similarity(&locals[0].1, &profiles[2].1);
        let raw_runner = cosine_similarity(&locals[0].1, &profiles[3].1);
        assert!(
            raw_best > 0.98 && (raw_best - raw_runner) < MATCH_MARGIN,
            "raw is ambiguous: best {raw_best} runner {raw_runner}"
        );

        // Centered scoring recovers the correct identities.
        let map = map_local_speakers_to_profiles(&locals, &profiles);
        assert_eq!(map.get("L2").map(String::as_str), Some("P2"));
        assert_eq!(map.get("L6").map(String::as_str), Some("P6"));
    }

    #[test]
    fn centered_regime_abstains_on_ambiguous_blend() {
        // A local sitting between two profiles (bumps on both axes) is not a
        // clear winner for either -> stays "Speaker N".
        let profiles = profiles8();
        let mut blended = vec![3.0f32; 8];
        blended[2] += 1.0; // between P2 ...
        blended[3] += 1.0; // ... and P3
        let locals = vec![("Lx".to_string(), unit(blended))];
        let map = map_local_speakers_to_profiles(&locals, &profiles);
        assert_eq!(map.get("Lx").map(String::as_str), Some("Speaker 1"));
    }
}
