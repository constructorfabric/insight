use std::path::Path;

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
        !self.full_clone && worth_purging(self.size_bytes, self.skeleton_bytes)
    }
}

/// Bytes held under `dir`, following subdirectories.
///
/// Blocking, recursive I/O: call it from `spawn_blocking` on any path where a
/// large tree is plausible. A path it cannot read counts as zero — a size it
/// cannot see must not be reported as a breach.
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

/// Whether an entry has taken on enough blob weight to be worth a repack.
///
/// The single definition of "drifted": the reclaim planner and the post-serve
/// purge must agree, or one of them repacks entries the other considers fine.
#[must_use]
pub fn worth_purging(size_bytes: u64, skeleton_bytes: u64) -> bool {
    skeleton_bytes > 0 && size_bytes > skeleton_bytes + skeleton_bytes / PURGE_MIN_GROWTH_DIVISOR
}

/// Packs an entry may accumulate before the serve-path purge consolidates
/// regardless of byte drift. Every page's prefetch writes at least one pack,
/// and git's object lookup pays a per-pack cost on every invocation — a
/// backfill of small blobs degrades linearly without ever tripping the byte
/// threshold. Well under git's own `gc.autoPackLimit` default (50), since
/// auto-maintenance is disabled on our invocations and this is what replaces
/// it.
const PACK_CONSOLIDATION_LIMIT: usize = 24;

/// Whether the serve-path purge should repack: byte drift over the skeleton,
/// or pack-count growth that byte drift cannot see.
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

/// Free bytes on the filesystem backing `path`, from `statvfs`.
///
/// The per-entry accounting is a LOWER bound on what the volume holds: it sees
/// only published entries, never a clone still staging under `tmp/`, and never
/// anything else sharing the volume. Effective free space is therefore the
/// minimum of the two views, and this is the second one.
///
/// `None` when the syscall fails — a missing view must not be read as "no
/// space", which would refuse every request.
#[must_use]
pub fn volume_available_bytes(path: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(path).ok()?;

    // `blocks_available` is what an unprivileged process may actually use;
    // `blocks_free` includes the reserved blocks it cannot touch.
    Some(stat.f_frsize.saturating_mul(stat.f_bavail))
}

impl Budget {
    /// `u64::MAX` is the "no budget" sentinel used by the test constructor.
    /// An unbounded budget makes every derived figure — watermarks, headroom,
    /// the volume comparison — meaningless, so callers must ask first.
    #[must_use]
    pub fn is_bounded(self) -> bool {
        self.total_bytes != u64::MAX
    }

    /// Usage as the budget sees it, given both views of free space.
    ///
    /// §3.6: effective free space is the MINIMUM of the per-entry accounting
    /// and the volume itself, so effective usage is the maximum of the two.
    /// The volume view is what notices a clone staging under `tmp/`, or
    /// another writer on the same mount.
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
    fn effective_usage_takes_the_stricter_of_the_two_views() {
        let budget = Budget {
            total_bytes: 1_000_000,
        };
        // The volume says 100k free, so 900k is in use — more than the
        // per-entry accounting can see (a clone staging under tmp/, say).
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
