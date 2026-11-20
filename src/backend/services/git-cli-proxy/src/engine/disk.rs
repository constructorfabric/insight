use std::path::Path;

const HIGH_WATERMARK_PCT: u64 = 85;
const LOW_WATERMARK_PCT: u64 = 65;
const PURGE_MIN_GROWTH_DIVISOR: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub dir_name: String,
    pub size_bytes: u64,
    pub skeleton_bytes: u64,
    pub last_accessed_at_epoch_s: u64,
    pub in_use: bool,
    pub full_clone: bool,
}

impl Candidate {
    fn reclaimable_by_purge(&self) -> u64 {
        self.size_bytes.saturating_sub(self.skeleton_bytes)
    }

    fn worth_purging(&self) -> bool {
        !self.full_clone && worth_purging(self.size_bytes, self.skeleton_bytes)
    }
}

#[must_use]
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            total += dir_size(&entry.path());
        } else if file_type.is_file() {
            total += entry.metadata().map_or(0, |m| m.len());
        }
    }
    total
}

#[must_use]
pub fn worth_purging(size_bytes: u64, skeleton_bytes: u64) -> bool {
    skeleton_bytes > 0 && size_bytes > skeleton_bytes + skeleton_bytes / PURGE_MIN_GROWTH_DIVISOR
}

pub(super) const PACK_CONSOLIDATION_LIMIT: usize = 24;

#[must_use]
pub fn needs_consolidation(size_bytes: u64, skeleton_bytes: u64, packs: usize) -> bool {
    worth_purging(size_bytes, skeleton_bytes) || packs >= PACK_CONSOLIDATION_LIMIT
}

#[cfg(test)]
mod consolidation_tests {
    use super::*;

    #[test]
    fn pack_growth_triggers_a_repack_that_byte_drift_cannot_see() {
        let cases = [
            ("no drift, few packs", 100, 100, 1, false),
            ("byte drift alone", 120, 100, 1, true),
            (
                "pack growth alone",
                100,
                100,
                PACK_CONSOLIDATION_LIMIT,
                true,
            ),
            (
                "one under the pack limit",
                100,
                100,
                PACK_CONSOLIDATION_LIMIT - 1,
                false,
            ),
        ];
        for (name, size, skeleton, packs, expected) in cases {
            assert_eq!(
                needs_consolidation(size, skeleton, packs),
                expected,
                "case: {name}"
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reclaim {
    PurgeBlobs { dir_name: String, frees: u64 },
    Evict { dir_name: String, frees: u64 },
}

impl Reclaim {
    #[must_use]
    pub fn dir_name(&self) -> &str {
        match self {
            Self::PurgeBlobs { dir_name, .. } | Self::Evict { dir_name, .. } => dir_name,
        }
    }

    fn frees(&self) -> u64 {
        match self {
            Self::PurgeBlobs { frees, .. } | Self::Evict { frees, .. } => *frees,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub total_bytes: u64,
}

#[must_use]
pub fn volume_available_bytes(path: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;

    // `f_bavail` is what an unprivileged process may actually use; `f_bfree`
    // includes the reserved blocks it cannot touch.
    Some(stat.f_frsize.saturating_mul(stat.f_bavail))
}

impl Budget {
    #[must_use]
    pub fn is_bounded(self) -> bool {
        self.total_bytes != u64::MAX
    }

    #[must_use]
    pub fn effective_used(self, accounted: u64, volume_available: Option<u64>) -> u64 {
        let Some(available) = volume_available.filter(|_| self.is_bounded()) else {
            return accounted;
        };
        accounted.max(self.total_bytes.saturating_sub(available))
    }

    #[must_use]
    pub fn high_watermark(self) -> u64 {
        self.total_bytes / 100 * HIGH_WATERMARK_PCT
    }

    #[must_use]
    pub fn low_watermark(self) -> u64 {
        self.total_bytes / 100 * LOW_WATERMARK_PCT
    }

    #[must_use]
    pub fn over_high_watermark(self, used: u64) -> bool {
        used > self.high_watermark()
    }

    #[must_use]
    pub fn excess_over_low(self, used: u64) -> u64 {
        used.saturating_sub(self.low_watermark())
    }
}

#[must_use]
pub fn plan_reclaim(candidates: &[Candidate], target_bytes: u64) -> Vec<Reclaim> {
    if target_bytes == 0 {
        return Vec::new();
    }

    let mut usable: Vec<&Candidate> = candidates.iter().filter(|c| !c.in_use).collect();
    usable.sort_by_key(|c| (c.last_accessed_at_epoch_s, c.dir_name.clone()));

    let mut plan: Vec<Reclaim> = Vec::new();
    let mut freed: u64 = 0;

    for candidate in &usable {
        if freed >= target_bytes {
            return plan;
        }
        if candidate.worth_purging() {
            let reclaimed = candidate.reclaimable_by_purge();
            freed += reclaimed;
            plan.push(Reclaim::PurgeBlobs {
                dir_name: candidate.dir_name.clone(),
                frees: reclaimed,
            });
        }
    }

    for candidate in &usable {
        if freed >= target_bytes {
            break;
        }
        let already_purged = plan
            .iter()
            .find(|step| step.dir_name() == candidate.dir_name)
            .map_or(0, Reclaim::frees);
        let reclaimed = candidate.size_bytes.saturating_sub(already_purged);
        freed += reclaimed;
        plan.retain(|step| step.dir_name() != candidate.dir_name);
        plan.push(Reclaim::Evict {
            dir_name: candidate.dir_name.clone(),
            frees: candidate.size_bytes,
        });
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, size: u64, skeleton: u64, accessed: u64) -> Candidate {
        Candidate {
            dir_name: name.to_owned(),
            size_bytes: size,
            skeleton_bytes: skeleton,
            last_accessed_at_epoch_s: accessed,
            in_use: false,
            full_clone: false,
        }
    }

    #[test]
    fn effective_usage_takes_the_stricter_of_the_two_views() {
        let budget = Budget {
            total_bytes: 1_000_000,
        };
        assert_eq!(
            budget.effective_used(400_000, Some(100_000)),
            900_000,
            "the volume view must win when it is stricter"
        );
        assert_eq!(
            budget.effective_used(900_000, Some(800_000)),
            900_000,
            "the accounting view must win when it is stricter"
        );
    }

    #[test]
    fn a_missing_volume_reading_falls_back_to_the_accounting() {
        let budget = Budget {
            total_bytes: 1_000_000,
        };
        assert_eq!(
            budget.effective_used(400_000, None),
            400_000,
            "a failed statvfs must not be read as a full volume"
        );
    }

    #[test]
    fn an_unbounded_budget_ignores_the_volume() {
        let budget = Budget {
            total_bytes: u64::MAX,
        };
        assert!(!budget.is_bounded());
        assert_eq!(
            budget.effective_used(42, Some(1)),
            42,
            "with no budget every derived figure is meaningless, including this one"
        );
    }

    #[test]
    fn watermarks_leave_a_hysteresis_band() {
        let budget = Budget {
            total_bytes: 1_000_000,
        };
        assert_eq!(budget.high_watermark(), 850_000);
        assert_eq!(budget.low_watermark(), 650_000);
        assert!(
            !budget.over_high_watermark(850_000),
            "the mark itself is fine"
        );
        assert!(budget.over_high_watermark(850_001));
        assert_eq!(budget.excess_over_low(900_000), 250_000);
        assert_eq!(
            budget.excess_over_low(100),
            0,
            "under the mark, nothing to do"
        );
    }

    #[test]
    fn purge_comes_before_eviction() {
        let candidates = vec![candidate("fat", 1_000, 100, 10)];
        let plan = plan_reclaim(&candidates, 500);
        assert_eq!(
            plan,
            vec![Reclaim::PurgeBlobs {
                dir_name: "fat".to_owned(),
                frees: 900
            }],
            "a repo carrying blobs is purged, not deleted"
        );
    }

    #[test]
    fn eviction_follows_lru_order_when_purging_is_not_enough() {
        let candidates = vec![
            candidate("newest", 1_000, 1_000, 300),
            candidate("oldest", 1_000, 1_000, 100),
            candidate("middle", 1_000, 1_000, 200),
        ];
        let plan = plan_reclaim(&candidates, 1_500);

        let names: Vec<&str> = plan.iter().map(Reclaim::dir_name).collect();
        assert_eq!(names, vec!["oldest", "middle"], "least recent goes first");
        assert!(
            plan.iter()
                .all(|step| matches!(step, Reclaim::Evict { .. })),
            "skeleton-sized repos have nothing to purge: {plan:?}"
        );
    }

    #[test]
    fn a_full_clone_entry_is_never_purged_only_evicted() {
        let mut promoted = candidate("promoted", 10_000, 1_000, 1);
        promoted.full_clone = true;
        let candidates = vec![promoted];

        let plan = plan_reclaim(&candidates, 5_000);
        assert_eq!(
            plan,
            vec![Reclaim::Evict {
                dir_name: "promoted".to_owned(),
                frees: 10_000
            }]
        );
    }

    #[test]
    fn in_use_entries_are_never_reclaimed() {
        let mut pinned = candidate("pinned", 10_000, 100, 1);
        pinned.in_use = true;
        let candidates = vec![pinned, candidate("free", 1_000, 1_000, 999)];

        let plan = plan_reclaim(&candidates, 100_000);
        let names: Vec<&str> = plan.iter().map(Reclaim::dir_name).collect();
        assert_eq!(names, vec!["free"], "a repo in use is untouchable");
    }

    #[test]
    fn a_purged_repo_that_must_also_go_is_planned_once() {
        let candidates = vec![candidate("only", 1_000, 100, 5)];
        let plan = plan_reclaim(&candidates, 100_000);
        assert_eq!(
            plan,
            vec![Reclaim::Evict {
                dir_name: "only".to_owned(),
                frees: 1_000
            }],
            "the entry is deleted once, not purged and deleted"
        );
    }

    #[test]
    fn nothing_to_reclaim_yields_an_empty_plan() {
        let cases = vec![
            ("no target", vec![candidate("a", 100, 10, 1)], 0),
            ("no candidates", Vec::new(), 1_000),
        ];
        for (name, candidates, target) in cases {
            assert!(plan_reclaim(&candidates, target).is_empty(), "case: {name}");
        }
    }

    #[test]
    fn a_marginally_grown_repo_is_not_worth_a_repack() {
        let barely = candidate("barely", 105, 100, 1);
        assert!(!barely.worth_purging(), "5% growth does not earn a repack");
        let grown = candidate("grown", 200, 100, 1);
        assert!(grown.worth_purging());
    }
}
