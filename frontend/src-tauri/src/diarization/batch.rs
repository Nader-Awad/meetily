// diarization/batch.rs
//
// Pure turn→unit merging and speakrs-speaker→saved-profile mapping for the
// batch (Retranscribe/Import) diarization path. No I/O, no model calls here —
// callers supply CAM++ centroids; this module is fully unit-testable.

use super::clustering::cosine_similarity;
use super::segmenter::DiarTurn;
use std::collections::HashMap;

pub const MIN_UNIT_MS: u64 = 1000;
pub const MERGE_GAP_MS: u64 = 500;

/// A cluster only adopts a saved profile's name when it is a CLEAR winner:
/// nearest profile, cosine ≥ HIGH_MATCH_THRESHOLD, and ahead of the runner-up
/// by ≥ MATCH_MARGIN. Otherwise the cluster stays "Speaker N" (unknown) — we
/// never force a guess. Deliberately stricter than the intra-meeting
/// clustering threshold; a weak/ambiguous match must read as unknown.
pub const HIGH_MATCH_THRESHOLD: f32 = 0.72;
pub const MATCH_MARGIN: f32 = 0.08;

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

/// Map each local speakrs speaker to a saved profile name, but only when the
/// match is a clear, confident win (see `HIGH_MATCH_THRESHOLD`/`MATCH_MARGIN`).
/// Otherwise falls back to a stable "Speaker N" label (numbered by first
/// appearance).
pub fn map_local_speakers_to_profiles(
    local_centroids: &[(String, Vec<f32>)],
    profiles: &[(String, Vec<f32>)],
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut anon: usize = 0;
    for (local, centroid) in local_centroids {
        let mut sims: Vec<(&String, f32)> = profiles
            .iter()
            .map(|(name, emb)| (name, cosine_similarity(centroid, emb)))
            .collect();
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let confident = match sims.first() {
            Some((name, best)) => {
                let runner_up = sims.get(1).map(|(_, s)| *s).unwrap_or(0.0);
                if *best >= HIGH_MATCH_THRESHOLD && (*best - runner_up) >= MATCH_MARGIN {
                    Some((*name).clone())
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
}
