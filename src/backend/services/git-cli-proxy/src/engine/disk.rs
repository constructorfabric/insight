const HIGH_WATERMARK_PCT: u64 = 85;
const LOW_WATERMARK_PCT: u64 = 65;
/// A repo must have grown by at least this fraction of its blobless baseline
/// before a repack is worth its cost.
const PURGE_MIN_GROWTH_DIVISOR: u64 = 10;

/// What the cache knows about one entry when deciding what to drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub dir_name: String,
    pub size_bytes: u64,
    /// Size right after a clone or a purge — the blobless skeleton. Anything
    /// above it is transient blob weight a purge can reclaim.
    pub skeleton_bytes: u64,
    pub last_accessed_at_epoch_s: u64,
    /// A repo with readers or a running clone must not be touched.
    pub in_use: bool,
    /// Promoted out of a partial clone because origin refuses explicit object
    /// requests. Purging its blobs would strand the entry: they cannot be
    /// fetched back.
    pub full_clone: bool,
}

impl Candidate {
    fn reclaimable_by_purge(&self) -> u64 {
        self.size_bytes.saturating_sub(self.skeleton_bytes)
    }

    fn worth_purging(&self) -> bool {
        !self.full_clone
            && self.skeleton_bytes > 0
            && self.size_bytes
                > self.skeleton_bytes + self.skeleton_bytes / PURGE_MIN_GROWTH_DIVISOR
    }
}

/// One step of a reclaim plan. Blob purge first — it keeps the repo warm, so
/// the next sync fetches instead of re-cloning.
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

impl Budget {
    #[must_use]
    pub fn high_watermark(self) -> u64 {
        self.total_bytes / 100 * HIGH_WATERMARK_PCT
    }

    #[must_use]
    pub fn low_watermark(self) -> u64 {
        self.total_bytes / 100 * LOW_WATERMARK_PCT
    }

    /// Whether `used` has crossed the point where reclaiming starts. Hysteresis
    /// is the point: reclaim runs to the LOW mark, so the next request does not
    /// immediately trip the HIGH mark again.
    #[must_use]
    pub fn over_high_watermark(self, used: u64) -> bool {
        used > self.high_watermark()
    }

    /// Bytes to reclaim to get back to the low watermark.
    #[must_use]
    pub fn excess_over_low(self, used: u64) -> u64 {
        used.saturating_sub(self.low_watermark())
    }
}

/// Order candidates least-recently-used first and pick reclaim steps until
/// `target_bytes` are freed: blob purges first (cheap, keeps the repo warm),
/// then whole-entry evictions. In-use entries are never touched — a reader
/// must never observe a partially deleted repository.
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

        // Purging would free blobs the origin refuses to serve again, which
        // would strand the entry: only a whole eviction is safe.
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
