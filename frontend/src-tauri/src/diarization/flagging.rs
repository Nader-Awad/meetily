// diarization/flagging.rs
//
// Detect saved voice profiles that are likely contaminated or mutually
// confusable, so the user can review and prune them. A profile is "confusable"
// when its closest exemplar to some OTHER saved voice is still high AFTER
// anisotropy correction (centered cosine) — meaning the two would frequently be
// mislabeled for each other. That is usually a sign that one profile was
// contaminated by another person's audio, or that two voices are genuinely too
// similar to separate. Pure (no I/O); mirrors the matcher's centering.

use super::clustering::cosine_similarity;
use super::normalize::{center_normalized, cohort_mean};

/// A saved profile that collides with another saved voice.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfusableFlag {
    /// The flagged profile.
    pub name: String,
    /// The other saved voice it is closest to.
    pub confused_with: String,
    /// Centered cosine of the closest exemplar pair (higher = more confusable).
    pub score: f32,
}

/// Centered cosine at/above which two DIFFERENT saved voices are treated as
/// confusable. Just below the 0.60 auto-adopt bar: a pair this close in the
/// residual space will often be mislabeled for one another.
pub const CONFUSABLE_THRESHOLD: f32 = 0.55;

/// Minimum number of exemplars across all profiles before we trust the cohort
/// mean estimate enough to flag. Below this we return nothing rather than raise
/// false alarms (raw anisotropy would make everything look confusable).
pub const MIN_EXEMPLARS_FOR_FLAGGING: usize = 8;

/// Flag profiles that collide with another saved voice. `profiles` is
/// (name, exemplars). For each profile with at least one other profile within
/// `CONFUSABLE_THRESHOLD` (centered), reports its single worst collision.
/// Returns empty when there are too few profiles/exemplars to judge reliably.
pub fn flag_confusable_profiles(profiles: &[(String, Vec<Vec<f32>>)]) -> Vec<ConfusableFlag> {
    if profiles.len() < 2 {
        return Vec::new();
    }
    let cohort: Vec<&[f32]> = profiles
        .iter()
        .flat_map(|(_, ex)| ex.iter().map(|e| e.as_slice()))
        .collect();
    if cohort.len() < MIN_EXEMPLARS_FOR_FLAGGING {
        return Vec::new();
    }
    let mean = match cohort_mean(&cohort) {
        Some(m) => m,
        None => return Vec::new(),
    };

    // Pre-center every profile's exemplars once.
    let centered: Vec<Vec<Vec<f32>>> = profiles
        .iter()
        .map(|(_, ex)| ex.iter().map(|e| center_normalized(e, &mean)).collect())
        .collect();

    let mut flags = Vec::new();
    for i in 0..profiles.len() {
        let mut worst: Option<(usize, f32)> = None;
        for j in 0..profiles.len() {
            if i == j {
                continue;
            }
            // Closest exemplar pair between profile i and profile j.
            let mut best = f32::MIN;
            for a in &centered[i] {
                for b in &centered[j] {
                    let s = cosine_similarity(a, b);
                    if s > best {
                        best = s;
                    }
                }
            }
            if best >= CONFUSABLE_THRESHOLD && worst.map_or(true, |(_, w)| best > w) {
                worst = Some((j, best));
            }
        }
        if let Some((j, score)) = worst {
            flags.push(ConfusableFlag {
                name: profiles[i].0.clone(),
                confused_with: profiles[j].0.clone(),
                score,
            });
        }
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }

    /// Anisotropic 8-dim voice on a given axis (shared component + speaker bump).
    fn voice(axis: usize, jitter: f32) -> Vec<f32> {
        let mut v = vec![3.0f32; 8];
        v[axis] += 1.0 + jitter;
        unit(v)
    }

    #[test]
    fn distinct_voices_are_not_flagged() {
        // 8 clearly distinct voices (one axis each) -> none confusable.
        let profiles: Vec<(String, Vec<Vec<f32>>)> =
            (0..8).map(|i| (format!("P{i}"), vec![voice(i, 0.0)])).collect();
        assert!(flag_confusable_profiles(&profiles).is_empty());
    }

    #[test]
    fn a_contaminated_profile_is_flagged() {
        // 8 distinct voices, but "Bob" also carries an exemplar that is really
        // Alice's voice (same axis as Alice) — Bob should be flagged as confusable
        // with Alice (and vice-versa).
        let mut profiles: Vec<(String, Vec<Vec<f32>>)> =
            (0..7).map(|i| (format!("P{i}"), vec![voice(i, 0.0)])).collect();
        profiles.push(("Alice".to_string(), vec![voice(7, 0.0)]));
        // Bob's own voice is axis 6-ish, but a contaminated exemplar sits on
        // Alice's axis 7.
        profiles.push(("Bob".to_string(), vec![voice(6, 0.3), voice(7, 0.05)]));

        let flags = flag_confusable_profiles(&profiles);
        let names: Vec<&str> = flags.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"Bob"), "Bob should be flagged: {flags:?}");
        assert!(
            flags.iter().any(|f| f.name == "Bob" && f.confused_with == "Alice"),
            "Bob should collide with Alice: {flags:?}"
        );
    }

    #[test]
    fn too_few_exemplars_returns_empty() {
        let profiles = vec![
            ("A".to_string(), vec![voice(0, 0.0)]),
            ("B".to_string(), vec![voice(0, 0.02)]), // identical-ish but too few overall
        ];
        assert!(flag_confusable_profiles(&profiles).is_empty());
    }
}
