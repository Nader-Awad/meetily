// diarization/batch.rs
//
// Pure turn→unit merging and speakrs-speaker→saved-profile mapping for the
// batch (Retranscribe/Import) diarization path. No I/O, no model calls here —
// callers supply CAM++ centroids; this module is fully unit-testable.

use super::clustering::{cosine_similarity, PROFILE_MATCH_THRESHOLD};
use super::segmenter::DiarTurn;
use std::collections::HashMap;

pub const MIN_UNIT_MS: u64 = 1000;
pub const MERGE_GAP_MS: u64 = 500;

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

/// Map each local speakrs speaker to a saved profile name (cosine ≥ threshold)
/// or a stable "Speaker N" label (numbered by first appearance).
pub fn map_local_speakers_to_profiles(
    local_centroids: &[(String, Vec<f32>)],
    profiles: &[(String, Vec<f32>)],
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut anon: usize = 0;
    for (local, centroid) in local_centroids {
        let best = profiles
            .iter()
            .map(|(name, emb)| (name, cosine_similarity(centroid, emb)))
            .filter(|(_, sim)| *sim >= PROFILE_MATCH_THRESHOLD)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let label = match best {
            Some((name, _)) => name.clone(),
            None => {
                anon += 1;
                format!("Speaker {}", anon)
            }
        };
        map.insert(local.clone(), label);
    }
    map
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
}
