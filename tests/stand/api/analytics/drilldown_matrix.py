"""What each metric's evidence must add up to, one row per metric key.

Every metric in the registry is drilldown-declared: a source carries an
`evidence_ref` and a measure carries an `evidence_granularity`, both mandatory,
so capability is decided at run time from the health of the evidence relation
rather than per metric. Sweeping the catalogue is therefore the only way to
notice that one source's evidence has drifted from the observations derived from
it — the modal would show wrong numbers with every other test still green.

The sweep needs an expectation per metric, and it cannot be one rule: an
evidence row means something different at each granularity, and the period
scalar is a different aggregate per computation. `Tier` is that expectation, and
each variant is the STRONGEST statement provable from the serving path:

* observations are derived from evidence by the gold models, under the same
  scope predicate the drilldown compiler uses, so the two sides are the same
  rows aggregated twice;
* rows a person's identity did not resolve to reach neither side, so the two
  cannot disagree about which rows exist;
* `max`/`min` collapse across one person's several source accounts is the one
  place they legitimately can, which is why those metrics get an inequality and
  not an equality.

Kept as a literal rather than derived from a live response so that collection
stays offline, matching the rest of this suite, and so that adding a metric
without deciding what its evidence means is a failure rather than a silent gap.
`test_every_metric_definition_is_in_the_drilldown_matrix` is what enforces that.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from enum import StrEnum


class Tier(StrEnum):
    """The reconciliation a metric's evidence supports."""

    #: Event grain whose contribution is a constant 1 and which projects no
    #: value column: one row IS one unit of the metric.
    EXACT_COUNT = "exact_count"
    #: Additive measure summed into observations: the projected values sum to
    #: the period scalar.
    EXACT_SUM = "exact_sum"
    #: Median over event rows passed through to observations one-for-one, so
    #: the scalar is one of the projected values.
    EXACT_MEDIAN = "exact_median"
    #: Median over a measure whose value column is not projected, recoverable
    #: from the detail columns it was computed from.
    DERIVED_MEDIAN = "derived_median"
    #: Percentile over event rows passed through one-for-one, like
    #: `EXACT_MEDIAN` but at a quantile the expectation carries.
    EXACT_PERCENTILE = "exact_percentile"
    #: Ratio of two additive measures: the summed numerator over the summed
    #: denominator, scaled, then transformed.
    EXACT_RATIO = "exact_ratio"
    #: Distinct count whose counted subject is the day itself.
    EXACT_DISTINCT_DATES = "exact_distinct_dates"
    #: Sum of a day flag that collapses by `max` or `min` across a person's
    #: source accounts: evidence sums to at least the period scalar.
    COLLAPSE_BOUNDED_SUM = "collapse_bounded_sum"
    #: Ratio whose denominator is such a day flag: the evidence ratio is at
    #: most the period scalar.
    COLLAPSE_BOUNDED_RATIO = "collapse_bounded_ratio"
    #: Distinct count whose counted subject is projected nowhere; only the
    #: bound `1 <= period <= rows` survives.
    STRUCTURAL_ONLY = "structural_only"


@dataclass(frozen=True)
class Transform:
    """The affine-and-clamp a definition applies AFTER aggregation.

    It reaches no response — `scale` is on the wire but this is not — so
    reconciling a clamped metric means applying it here, in the same order the
    SQL does, and a metric pinned at its bound compares exactly.
    """

    multiplier: float = 1.0
    offset: float = 0.0
    clamp_min: float | None = None
    clamp_max: float | None = None

    def apply(self, value: float) -> float:
        transformed = self.multiplier * value + self.offset
        if self.clamp_min is not None:
            transformed = max(transformed, self.clamp_min)
        if self.clamp_max is not None:
            transformed = min(transformed, self.clamp_max)
        return transformed


@dataclass(frozen=True)
class Expectation:
    """One metric's drilldown expectation.

    `source` is the evidence family, and it is what makes a failure readable:
    capability and schema health are per source, so a whole family failing at
    once is a different defect from one metric failing alone.
    """

    metric_key: str
    source: str
    tier: Tier
    #: Ratio scale, from the definition's computation.
    scale: float | None = None
    #: Quantile of an `EXACT_PERCENTILE` metric, as the registry declares it.
    quantile: float | None = None
    transform: Transform | None = None
    #: Detail columns a `DERIVED_MEDIAN` metric's value is the sum of.
    derived_from: tuple[str, ...] = ()


_PERCENT = Transform(clamp_max=100.0)
_INVERTED_PERCENT = Transform(multiplier=-1.0, offset=100.0, clamp_min=0.0, clamp_max=100.0)

MATRIX: Sequence[Expectation] = (
    Expectation("ai.accepted_edit_actions", "ai", Tier.EXACT_SUM),
    Expectation("ai.accepted_lines", "ai", Tier.EXACT_SUM),
    Expectation("ai.active_days", "ai", Tier.COLLAPSE_BOUNDED_SUM),
    Expectation("ai.assistant_actions", "ai", Tier.EXACT_SUM),
    Expectation("ai.assistant_messages", "ai", Tier.EXACT_SUM),
    Expectation("ai.chat_assistant_conversations", "ai", Tier.EXACT_SUM),
    Expectation("ai.cost", "ai", Tier.EXACT_SUM),
    Expectation("ai.daily_approximate_extra_usage_cost", "ai_cost", Tier.EXACT_SUM),
    Expectation("ai.dev_conversations", "ai", Tier.EXACT_SUM),
    Expectation("ai.extra_usage_cost", "ai_cost", Tier.EXACT_SUM),
    Expectation("ai.extra_usage_utilisation", "ai_cost", Tier.EXACT_RATIO, scale=100.0),
    Expectation("ai.prs_total", "ai", Tier.EXACT_SUM),
    Expectation("ai.prs_with_assistant", "ai", Tier.EXACT_SUM),
    Expectation("ai.removed_lines", "ai", Tier.EXACT_SUM),
    Expectation("ai.seat_cost", "ai_cost", Tier.EXACT_SUM),
    Expectation("ai.tool_acceptance_rate", "ai", Tier.EXACT_RATIO, scale=100.0),
    Expectation("collab.active_days", "collab", Tier.EXACT_DISTINCT_DATES),
    Expectation("collab.adhoc_meetings", "collab", Tier.EXACT_SUM),
    Expectation("collab.breadth", "collab", Tier.STRUCTURAL_ONLY),
    Expectation("collab.channel_posts", "collab", Tier.EXACT_SUM),
    Expectation("collab.dm_ratio", "collab", Tier.EXACT_RATIO, scale=100.0),
    Expectation("collab.emails_read", "collab", Tier.EXACT_SUM),
    Expectation("collab.emails_received", "collab", Tier.EXACT_SUM),
    Expectation("collab.emails_sent", "collab", Tier.EXACT_SUM),
    Expectation("collab.files_engaged", "collab", Tier.EXACT_SUM),
    Expectation("collab.files_shared", "collab", Tier.EXACT_SUM),
    Expectation("collab.files_shared_external", "collab", Tier.EXACT_SUM),
    Expectation("collab.files_shared_internal", "collab", Tier.EXACT_SUM),
    Expectation("collab.focus_time_pct", "collab", Tier.EXACT_RATIO, scale=100.0),
    Expectation("collab.meeting_free_days", "collab", Tier.COLLAPSE_BOUNDED_SUM),
    Expectation("collab.meeting_hours", "collab", Tier.EXACT_SUM),
    Expectation("collab.meetings_count", "collab", Tier.EXACT_SUM),
    Expectation("collab.meetings_organized", "collab", Tier.EXACT_SUM),
    Expectation("collab.messages_sent", "collab", Tier.EXACT_SUM),
    Expectation("collab.msgs_per_active_day", "collab", Tier.COLLAPSE_BOUNDED_RATIO, scale=1.0),
    Expectation("collab.scheduled_meetings", "collab", Tier.EXACT_SUM),
    Expectation("git.active_days", "git", Tier.EXACT_DISTINCT_DATES),
    Expectation("git.approval_to_merge_time_h", "git", Tier.EXACT_MEDIAN),
    Expectation("git.code_lines", "git", Tier.EXACT_SUM),
    Expectation(
        "git.commit_size",
        "git",
        Tier.DERIVED_MEDIAN,
        derived_from=("lines_added", "lines_removed"),
    ),
    Expectation("git.commits", "git", Tier.EXACT_COUNT),
    Expectation("git.commits_per_active_day", "git", Tier.EXACT_RATIO, scale=1.0),
    Expectation("git.first_review_time_p75_h", "git", Tier.EXACT_PERCENTILE, quantile=0.75),
    Expectation("git.first_review_time_h", "git", Tier.EXACT_MEDIAN),
    Expectation("git.default_branch_code_lines", "git", Tier.EXACT_SUM),
    Expectation("git.default_branch_commits", "git", Tier.EXACT_COUNT),
    Expectation("git.default_branch_lines_added", "git", Tier.EXACT_SUM),
    Expectation("git.default_branch_lines_removed", "git", Tier.EXACT_SUM),
    Expectation("git.default_branch_prs_created", "git", Tier.EXACT_COUNT),
    Expectation("git.default_branch_prs_merged", "git", Tier.EXACT_COUNT),
    Expectation("git.lines_added", "git", Tier.EXACT_SUM),
    Expectation("git.lines_removed", "git", Tier.EXACT_SUM),
    Expectation("git.merge_rate", "git", Tier.EXACT_RATIO, scale=100.0),
    Expectation("git.non_default_branch_code_lines", "git", Tier.EXACT_SUM),
    Expectation("git.non_default_branch_commits", "git", Tier.EXACT_COUNT),
    Expectation("git.non_default_branch_lines_added", "git", Tier.EXACT_SUM),
    Expectation("git.non_default_branch_lines_removed", "git", Tier.EXACT_SUM),
    Expectation("git.non_default_branch_prs_created", "git", Tier.EXACT_COUNT),
    Expectation("git.non_default_branch_prs_merged", "git", Tier.EXACT_COUNT),
    Expectation("git.merges_without_approval_rate", "git", Tier.EXACT_RATIO, scale=100.0),
    Expectation("git.multi_reviewer_rate", "git", Tier.EXACT_RATIO, scale=100.0),
    Expectation("git.pr_abandonment_rate", "git", Tier.EXACT_RATIO, scale=100.0),
    Expectation("git.pr_comments", "git", Tier.EXACT_COUNT),
    Expectation("git.pr_commits", "git", Tier.EXACT_MEDIAN),
    Expectation("git.pr_cycle_time_h", "git", Tier.EXACT_MEDIAN),
    Expectation("git.pr_cycle_time_p75_h", "git", Tier.EXACT_PERCENTILE, quantile=0.75),
    Expectation("git.pr_size", "git", Tier.EXACT_MEDIAN),
    Expectation("git.prs_created", "git", Tier.EXACT_COUNT),
    Expectation("git.prs_merged", "git", Tier.EXACT_COUNT),
    Expectation("git.review_coverage", "git", Tier.EXACT_RATIO, scale=100.0),
    Expectation("git.reviews_performed", "git", Tier.EXACT_COUNT),
    Expectation("git.review_to_merge_time_h", "git", Tier.EXACT_MEDIAN),
    Expectation("git.review_wait_share", "git", Tier.EXACT_MEDIAN),
    Expectation("git.reviewers_per_pr", "git", Tier.EXACT_RATIO, scale=1.0),
    Expectation("git.test_change_share", "git", Tier.EXACT_RATIO, scale=100.0),
    Expectation("tasks.avg_slip", "task", Tier.EXACT_RATIO, scale=1.0),
    Expectation("tasks.bugs_fixed", "task", Tier.EXACT_COUNT),
    Expectation("tasks.bugs_ratio", "task", Tier.EXACT_RATIO, scale=100.0),
    Expectation("tasks.closed", "task", Tier.EXACT_COUNT),
    Expectation("tasks.closed_non_bug", "task", Tier.EXACT_COUNT),
    Expectation("tasks.dev_time", "task", Tier.EXACT_MEDIAN),
    Expectation("tasks.due_date_compliance", "task", Tier.EXACT_RATIO, scale=100.0),
    Expectation(
        "tasks.estimation_accuracy",
        "task",
        Tier.EXACT_RATIO,
        scale=1.0,
        transform=_INVERTED_PERCENT,
    ),
    Expectation("tasks.flow_efficiency", "task", Tier.EXACT_RATIO, scale=100.0, transform=_PERCENT),
    Expectation("tasks.on_time_delivery", "task", Tier.EXACT_RATIO, scale=100.0),
    Expectation("tasks.pickup_time", "task", Tier.EXACT_MEDIAN),
    Expectation("tasks.reopen_rate", "task", Tier.EXACT_RATIO, scale=100.0),
    Expectation("tasks.resolution_time", "task", Tier.EXACT_MEDIAN),
    Expectation("tasks.stale_in_progress", "task", Tier.EXACT_SUM),
    Expectation(
        "tasks.worklog_accuracy", "task", Tier.EXACT_RATIO, scale=100.0, transform=_PERCENT
    ),
    Expectation("wiki.comments", "wiki", Tier.EXACT_SUM),
    Expectation("wiki.edits", "wiki", Tier.EXACT_SUM),
    Expectation("wiki.pages_created", "wiki", Tier.EXACT_COUNT),
    Expectation("wiki.pages_edited", "wiki", Tier.EXACT_SUM),
)

#: A metric the stand serves but has no evidence for: its measure reads
#: `class_collab_document_activity`, which no generator writes, so the drilldown
#: answers 200 with no rows. Named once because two tests need the same
#: property, and it holds only as long as that relation stays unseeded.
EMPTY_EVIDENCE_METRIC = "collab.files_engaged"

#: One metric per distinct evidence presentation, plus the capable-but-empty
#: case. A presentation is all an export can differ by — the column set and the
#: header labels are everything it serializes — and every other metric in the
#: catalogue reuses one of these, so exporting the whole catalogue would repeat
#: these answers rather than add any.
EXPORT_SHAPES: Sequence[str] = (
    "git.prs_created",
    "git.pr_cycle_time_h",
    "tasks.closed",
    "tasks.dev_time",
    "git.merge_rate",
    "collab.messages_sent",
    "wiki.pages_created",
    EMPTY_EVIDENCE_METRIC,
)
