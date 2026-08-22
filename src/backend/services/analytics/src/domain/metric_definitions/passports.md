# Metric passports

Generated from `registry.yaml` by `analytics passports`. Do not edit by hand —
regenerate and commit. A drift test (`metric_definitions::passport`) fails when
this file and the registry disagree.

## ai.accepted_lines — AI-added lines

- Source: ai_usage (ai_metric_observations)
- Reads: accepted_lines
- Formula: sum(accepted_lines)
- Shape: integer, higher_is_better, unit lines
- Notes: Accepted AI-generated added lines across coding AI tools.

## ai.removed_lines — AI-removed lines

- Source: ai_usage (ai_metric_observations)
- Reads: removed_lines
- Formula: sum(removed_lines)
- Shape: integer, higher_is_better, unit lines
- Notes: Accepted AI-generated removed lines across coding AI tools.

## ai.active_days — AI active days

- Source: ai_usage (ai_metric_observations)
- Reads: active_day
- Formula: sum(active_day)
- Shape: integer, higher_is_better, unit days
- Notes: Distinct days with person-attributed AI activity across dev and assistant tools.

## ai.cost — AI potential usage cost

- Source: ai_usage (ai_metric_observations)
- Reads: cost_usd
- Formula: sum(cost_usd)
- Shape: currency, lower_is_better
- Notes: Person-attributed AI usage priced at the vendor's token rates — what the consumption would cost if billed purely by usage. It includes usage a seat already covered and excludes seat fees, so it is not the invoiced amount, and only tools that price usage per person contribute. Never add it to actual usage cost, which is the billed part of this same consumption.

## ai.seat_cost — AI seat cost

- Source: ai_cost (ai_cost_metric_observations)
- Reads: seat_cost_usd
- Formula: sum(seat_cost_usd)
- Shape: currency, lower_is_better
- Notes: The invoiced price of one seat for a billing month, read from the per-seat amount on the invoice. A monthly figure, so a partial window returns the whole month and a window spanning two months returns both fees. A seat whose tier the invoice does not price returns no value rather than a share of the total. Add actual usage cost for the full cost of a seat.

## ai.extra_usage_cost — AI actual usage cost

- Source: ai_cost (ai_cost_metric_observations)
- Reads: extra_usage_usd
- Formula: sum(extra_usage_usd)
- Shape: currency, lower_is_better
- Notes: What the vendor billed on top of the seat fee, once the usage that fee covered was exhausted, priced at API rates. A monthly figure, so a window covering part of a month returns the whole month rather than a fraction. This is the exact billed amount; its per-day distribution approximates the same money, and neither adds to potential usage cost.

## ai.daily_approximate_extra_usage_cost — AI actual usage cost — approximate distribution

- Source: ai_cost (ai_cost_metric_observations)
- Reads: daily_extra_usage_usd
- Formula: sum(daily_extra_usage_usd)
- Shape: currency, lower_is_better
- Notes: The billed extra-usage cost placed on the days it was spent. The vendor reports only a running month-to-date total, so a day's figure is the step between two readings — exact in sum over a month, approximate in placement. A day with no reading shows no point, and a correction never produces a negative day.

## ai.extra_usage_utilisation — Extra-usage ceiling used

- Source: ai_cost (ai_cost_metric_observations)
- Reads: extra_usage_usd, extra_usage_limit_usd
- Formula: 100 * (extra_usage_usd / extra_usage_limit_usd)
- Shape: percent, lower_is_better
- Notes: Extra usage measured against the ceiling in force on the seat — the plan tier's default, or a different one set for that member. At 100 per cent the vendor stops the seat, so this reads as proximity to being blocked, not as waste — room left under a ceiling was never purchased and costs nothing. A seat with no ceiling returns no value rather than a zero, the ratio having no denominator. Above 100 per cent means the ceiling was lowered below what the seat had already spent — the vendor reports the ceiling in force now against the whole month's spend, so a limit set or reduced mid-month is read against money that predates it.

## ai.accepted_edit_actions — Accepted AI edits

- Source: ai_usage (ai_metric_observations)
- Reads: accepted_edit_actions
- Formula: sum(accepted_edit_actions)
- Shape: integer, higher_is_better, unit actions
- Notes: Accepted AI edit or tool suggestions across supported coding AI tools.

## ai.tool_acceptance_rate — AI tool acceptance

- Source: ai_usage (ai_metric_observations)
- Reads: accepted_edit_actions, tool_use_offered
- Formula: 100 * (accepted_edit_actions / tool_use_offered)
- Shape: percent, higher_is_better
- Notes: Accepted AI edit or tool suggestions divided by offered suggestions.

## ai.assistant_messages — AI assistant messages

- Source: ai_usage (ai_metric_observations)
- Reads: assistant_messages
- Formula: sum(assistant_messages)
- Shape: integer, higher_is_better, unit messages
- Notes: Person-attributed assistant messages from supported AI assistant tools.

## ai.assistant_actions — AI assistant actions

- Source: ai_usage (ai_metric_observations)
- Reads: assistant_actions
- Formula: sum(assistant_actions)
- Shape: integer, higher_is_better, unit actions
- Notes: Person-attributed assistant actions from supported AI assistant tools.

## ai.dev_conversations — AI dev conversations

- Source: ai_usage (ai_metric_observations)
- Reads: dev_conversations
- Formula: sum(dev_conversations)
- Shape: integer, higher_is_better, unit conversations
- Notes: Person-attributed conversations with coding AI tools, counted as the vendor counts them. For the agent tools this is the session or thread count — a session is the unit of conversation there, and no vendor publishes a separate conversation counter — so the number reads as "times the person started working with the assistant", not as messages exchanged. Tools that report no such unit, inline-completion tools among them, return no value rather than a zero.

## ai.chat_assistant_conversations — AI chat conversations

- Source: ai_usage (ai_metric_observations)
- Reads: chat_assistant_conversations
- Formula: sum(chat_assistant_conversations)
- Shape: integer, higher_is_better, unit conversations
- Notes: Person-attributed chat assistant conversations from supported AI chat tools.

## ai.prs_with_assistant — PRs with AI assistance

- Source: ai_usage (ai_metric_observations)
- Reads: prs_with_assistant
- Formula: sum(prs_with_assistant)
- Shape: integer, higher_is_better, unit PRs
- Notes: Pull requests where the coding assistant was active at least once, as the vendor attributes them. Reported only by sources that connect to the code host themselves, so a person working without that connection returns no value rather than a zero. Counts pull requests, not commits or lines, and says nothing about how much of the change the assistant wrote.

## ai.prs_total — PRs seen by the AI vendor

- Source: ai_usage (ai_metric_observations)
- Reads: prs_total
- Formula: sum(prs_total)
- Shape: integer, neutral, unit PRs
- Notes: Pull requests the AI vendor observed for the person, served as the context for PRs with AI assistance rather than as a goal of its own. It is the vendor's count over the vendor's own window, which need not be the day it is reported against and need not agree with the git sources — read it next to that measure, not next to git pull-request metrics.

## git.commits — Commits

- Source: git (git_metric_observations)
- Reads: commit_count
- Formula: sum(commit_count)
- Shape: integer, higher_is_better, unit commits
- Notes: Distinct authored commits across connected git sources, excluding merge commits.

## git.code_lines — Code lines added

- Source: git (git_metric_observations)
- Reads: code_lines_added
- Formula: sum(code_lines_added)
- Shape: integer, higher_is_better, unit lines
- Notes: Lines added to files classified as code — tests, configuration, and documentation excluded. Each change counts once: when the same content reaches a repository in more than one commit, the lines belong to the commit that introduced them first.

## git.lines_added — Lines added

- Source: git (git_metric_observations)
- Reads: lines_added
- Formula: sum(lines_added)
- Shape: integer, higher_is_better, unit lines
- Notes: Lines added across all files, split by file category: code, tests, configuration, documentation. Each change counts once: when the same content reaches a repository in more than one commit, the lines belong to the commit that introduced them first.

## git.test_change_share — Test change share

- Source: git (git_metric_observations)
- Reads: test_lines_added, test_and_code_lines_added
- Formula: 100 * (test_lines_added / test_and_code_lines_added)
- Shape: percent, neutral
- Notes: Lines added to test files divided by lines added to code and test files. Documentation, configuration, and generated files do not affect the percentage. A result near zero can identify changes where tests are not evolving with implementation, but expected levels vary by repository and change type.

## git.lines_removed — Lines removed

- Source: git (git_metric_observations)
- Reads: lines_removed
- Formula: sum(lines_removed)
- Shape: integer, neutral, unit lines
- Notes: Lines removed across all reported file changes, with file-category, repository, and source breakdowns available. Each change counts once: when the same removal reaches a repository in more than one commit, the lines belong to the commit that made it first.

## git.prs_created — Pull requests created

- Source: git (git_metric_observations)
- Reads: pr_created
- Formula: sum(pr_created)
- Shape: integer, higher_is_better, unit PRs
- Notes: Pull requests opened, dated by creation.

## git.prs_merged — Pull requests merged

- Source: git (git_metric_observations)
- Reads: pr_merged
- Formula: sum(pr_merged)
- Shape: integer, higher_is_better, unit PRs
- Notes: Authored pull requests that merged, dated by the merge.

## git.merge_rate — PR merge rate

- Source: git (git_metric_observations)
- Reads: pr_created_merged, pr_created
- Formula: 100 * (pr_created_merged / pr_created)
- Shape: percent, higher_is_better
- Notes: Of the pull requests created in the period, the share that have merged. Requests opened near the end of the period may not have merged yet, which lowers the rate at period edges.

## git.pr_abandonment_rate — PR abandonment rate

- Source: git (git_metric_observations)
- Reads: pr_abandoned, pr_created
- Formula: 100 * (pr_abandoned / pr_created)
- Shape: percent, lower_is_better
- Notes: Of pull requests created in the period, the share now closed without merging. Open requests are not abandoned. Requests created near the period end can still change outcome later.

## git.review_coverage — Review coverage

- Source: git (git_metric_observations)
- Reads: pr_reviewed, pr_created
- Formula: 100 * (pr_reviewed / pr_created)
- Shape: percent, higher_is_better
- Notes: Of pull requests created in the period, the share with at least one submitted review or approval. Assigned reviewers who never act do not count. Some sources provide approvals without review timestamps.

## git.reviewers_per_pr — Reviewers per PR

- Source: git (git_metric_observations)
- Reads: pr_reviewer_count, pr_created
- Formula: pr_reviewer_count / pr_created
- Shape: decimal, neutral
- Notes: Distinct reviewers who submitted a review or approval, divided by pull requests created in the period. Pull requests without review contribute zero, so read this with review coverage to separate breadth from coverage.

## git.multi_reviewer_rate — Multi-reviewer rate

- Source: git (git_metric_observations)
- Reads: pr_multi_reviewed, pr_reviewed
- Formula: 100 * (pr_multi_reviewed / pr_reviewed)
- Shape: percent, neutral
- Notes: Of pull requests with at least one submitted review or approval, the share with two or more distinct acting reviewers. This measures review breadth without conflating it with unreviewed pull requests.

## git.merges_without_approval_rate — Merges without approval

- Source: git (git_metric_observations)
- Reads: pr_merged_without_approval, pr_merged
- Formula: 100 * (pr_merged_without_approval / pr_merged)
- Shape: percent, lower_is_better
- Notes: Of pull requests merged in the period, the share with no approval reported by the connected source. This can reveal missing approval gates, but source configuration and incomplete review history can also affect it.

## git.active_days — Active commit days

- Source: git (git_metric_observations)
- Reads: commit_day
- Formula: distinct_count(commit_day)
- Shape: integer, neutral, unit days
- Notes: Distinct calendar days with authored, non-merge commits. Repository breakdowns count active days independently; the total still counts each calendar day once across repositories.

## git.commits_per_active_day — Commits per active day

- Source: git (git_metric_observations)
- Reads: commit_count, commit_day
- Formula: commit_count / distinct_count(commit_day)
- Shape: decimal, higher_is_better
- Notes: Commits divided by the number of days with at least one commit.

## git.commit_size — Commit size

- Source: git (git_metric_observations)
- Reads: commit_change_size
- Formula: median(commit_change_size)
- Shape: integer, lower_is_better, unit lines
- Notes: Median diff size of authored commits (lines added plus removed), counting only content a commit is the first to introduce — a commit that repeats content already in the repository has a size of zero. Smaller commits are easier to review.

## git.pr_size — PR size

- Source: git (git_metric_observations)
- Reads: pr_change_size
- Formula: median(pr_change_size)
- Shape: integer, lower_is_better, unit lines
- Notes: Median diff size of authored pull requests (lines added plus removed). Smaller requests are easier to review. Sources that do not report line counts contribute no values.

## git.pr_cycle_time_h — PR cycle time

- Source: git (git_metric_observations)
- Reads: pr_cycle_hours
- Formula: median(pr_cycle_hours)
- Shape: decimal, lower_is_better, unit h
- Notes: Median hours from opening a pull request to merging it, over requests merged in the period.

## git.first_review_time_h — Time to first review

- Source: git (git_metric_observations)
- Reads: pr_first_review_hours
- Formula: median(pr_first_review_hours)
- Shape: decimal, lower_is_better, unit h
- Notes: Median hours from opening a pull request to its first submitted review, over first reviews recorded in the period. Pull requests without a review and sources without review timestamps contribute no duration.

## git.review_wait_share — Review wait share

- Source: git (git_metric_observations)
- Reads: pr_review_wait_share
- Formula: median(pr_review_wait_share)
- Shape: percent, lower_is_better
- Notes: Median percentage of open-to-merge time elapsed before the first submitted review, over merged pull requests in the period. Sources without review timestamps contribute no value. High values suggest first review is the main cycle-time constraint.

## git.review_to_merge_time_h — Review-to-merge time

- Source: git (git_metric_observations)
- Reads: pr_review_to_merge_hours
- Formula: median(pr_review_to_merge_hours)
- Shape: decimal, lower_is_better, unit h
- Notes: Median hours from first submitted review to merge, over merged pull requests in the period. Sources without review timestamps contribute no value. Read with time to first review to locate delay before or after review starts.

## git.approval_to_merge_time_h — Approval-to-merge time

- Source: git (git_metric_observations)
- Reads: pr_approval_to_merge_hours
- Formula: median(pr_approval_to_merge_hours)
- Shape: decimal, lower_is_better, unit h
- Notes: Median hours from the latest reported approval with a timestamp to merge, over merged pull requests in the period. Sources that expose approvals without per-approval timestamps contribute no value. High values can identify delay after review gates clear.

## collab.messages_sent — Messages Sent

- Source: collab (collab_metric_observations)
- Reads: total_chat_messages
- Formula: sum(total_chat_messages)
- Shape: integer, higher_is_better, unit messages
- Notes: Chat messages a person sent across messaging tools. Counts are not directly comparable between tools: Slack includes thread replies, and Microsoft 365 combines private-chat and channel messages.

## collab.channel_posts — Channel Posts

- Source: collab (collab_metric_observations)
- Reads: channel_posts
- Formula: sum(channel_posts)
- Shape: integer, higher_is_better, unit messages
- Notes: Channel posts plus thread replies across messaging tools. Tools that report posts and replies separately are folded so counts stay comparable.

## collab.dm_ratio — DM Ratio

- Source: collab (collab_metric_observations)
- Reads: direct_and_group_messages, total_chat_messages
- Formula: 100 * (direct_and_group_messages / total_chat_messages)
- Shape: percent, lower_is_better
- Notes: Direct and group-chat messages divided by all chat messages. A lower ratio means more communication happens in open channels. Tools that do not distinguish message types report no value.

## collab.msgs_per_active_day — Messages per Active Day

- Source: collab (collab_metric_observations)
- Reads: total_chat_messages, chat_active_day
- Formula: total_chat_messages / chat_active_day
- Shape: decimal, higher_is_better, unit messages/day
- Notes: Chat messages sent divided by days with chat messages. Each tool's active days count separately.

## collab.active_days — Active Days

- Source: collab (collab_metric_observations)
- Reads: active_day
- Formula: distinct_count(active_day)
- Shape: integer, higher_is_better, unit days
- Notes: Distinct days on which a person took a deliberate collaboration action — sending a message, sending email, engaging or sharing a file, or attending a meeting. Passive activity such as receiving or reading email is excluded.

## collab.emails_sent — Emails Sent

- Source: collab (collab_metric_observations)
- Reads: emails_sent
- Formula: sum(emails_sent)
- Shape: integer, higher_is_better, unit emails
- Notes: Emails a person sent.

## collab.emails_received — Emails Received

- Source: collab (collab_metric_observations)
- Reads: emails_received
- Formula: sum(emails_received)
- Shape: integer, higher_is_better, unit emails
- Notes: Emails a person received.

## collab.emails_read — Emails Read

- Source: collab (collab_metric_observations)
- Reads: emails_read
- Formula: sum(emails_read)
- Shape: integer, higher_is_better, unit emails
- Notes: Emails a person read.

## collab.files_engaged — Files Engaged

- Source: collab (collab_metric_observations)
- Reads: files_engaged
- Formula: sum(files_engaged)
- Shape: integer, higher_is_better, unit files
- Notes: Files a person viewed or edited.

## collab.files_shared_internal — Files Shared (Internal)

- Source: collab (collab_metric_observations)
- Reads: files_shared_internal
- Formula: sum(files_shared_internal)
- Shape: integer, higher_is_better, unit files
- Notes: Files a person shared with people inside the organization.

## collab.files_shared_external — Files Shared (External)

- Source: collab (collab_metric_observations)
- Reads: files_shared_external
- Formula: sum(files_shared_external)
- Shape: integer, higher_is_better, unit files
- Notes: Files a person shared with people outside the organization.

## collab.files_shared — Files Shared

- Source: collab (collab_metric_observations)
- Reads: files_shared
- Formula: sum(files_shared)
- Shape: integer, higher_is_better, unit files
- Notes: Files a person shared with recipients inside or outside the organization.

## collab.meeting_hours — Meeting Hours

- Source: collab (collab_metric_observations)
- Reads: meeting_hours
- Formula: sum(meeting_hours)
- Shape: decimal, lower_is_better, unit h
- Notes: Hours spent in meetings, taking the longest active modality (audio, video, or screen share) per meeting. Zoom reports modality durations as full-session estimates, so its figures may run higher than Microsoft Teams.

## collab.meetings_count — Meetings Attended

- Source: collab (collab_metric_observations)
- Reads: meetings_attended
- Formula: sum(meetings_attended)
- Shape: integer, higher_is_better, unit meetings
- Notes: Distinct meetings a person attended across meeting tools.

## collab.meeting_free_days — Meeting-Free Days

- Source: collab (collab_metric_observations)
- Reads: meeting_free_day
- Formula: sum(meeting_free_day)
- Shape: integer, higher_is_better, unit days
- Notes: Days on which a person was actively collaborating but spent no time in meetings — a proxy for uninterrupted working days.

## collab.focus_time_pct — Focus Time

- Source: collab (collab_metric_observations)
- Reads: focus_hours, working_hours
- Formula: 100 * (focus_hours / working_hours)
- Shape: percent, higher_is_better
- Notes: Share of the workday not spent in meetings: meeting-free hours divided by scheduled working hours. Scheduled hours default to a nominal eight-hour day where an HR source does not provide them.

## collab.breadth — Collaboration Breadth

- Source: collab (collab_metric_observations)
- Reads: active_modality
- Formula: distinct_count(active_modality)
- Shape: integer, neutral, unit modalities
- Notes: Distinct collaboration modalities — chat, meetings, email, documents — a person was deliberately active in during the period.

## collab.meetings_organized — Meetings Organized

- Source: collab (collab_metric_observations)
- Reads: meetings_organized
- Formula: sum(meetings_organized)
- Shape: integer, neutral, unit meetings
- Notes: Meetings a person organized. Reported only by tools that expose organizer counts.

## collab.adhoc_meetings — Ad-hoc Meetings

- Source: collab (collab_metric_observations)
- Reads: adhoc_meetings_attended
- Formula: sum(adhoc_meetings_attended)
- Shape: integer, neutral, unit meetings
- Notes: Unscheduled meetings a person attended. Reported only by tools that distinguish ad-hoc from scheduled meetings.

## collab.scheduled_meetings — Scheduled Meetings

- Source: collab (collab_metric_observations)
- Reads: scheduled_meetings_attended
- Formula: sum(scheduled_meetings_attended)
- Shape: integer, neutral, unit meetings
- Notes: Scheduled meetings a person attended. Reported only by tools that distinguish ad-hoc from scheduled meetings.

## tasks.closed — Issues closed

- Source: task (task_metric_observations)
- Reads: tasks_closed
- Formula: sum(tasks_closed)
- Shape: integer, higher_is_better, unit issues
- Notes: All issues a person moved into a closed status during the period. Bugs are part of this number and are listed separately by type.

## tasks.bugs_fixed — Bugs closed

- Source: task (task_metric_observations)
- Reads: bugs_fixed
- Formula: sum(bugs_fixed)
- Shape: integer, higher_is_better, unit issues
- Notes: Issues of a bug type a person closed during the period. Part of issues closed, not a separate total.

## tasks.closed_non_bug — Non-bug issues closed

- Source: task (task_metric_observations)
- Reads: closed_non_bug
- Formula: sum(closed_non_bug)
- Shape: integer, higher_is_better, unit issues
- Notes: Issues of a known non-bug type a person closed during the period. Issues whose type cannot be determined are excluded rather than counted here.

## tasks.dev_time — Development time

- Source: task (task_metric_observations)
- Reads: dev_time_hours
- Formula: median(dev_time_hours)
- Shape: decimal, lower_is_better, unit h
- Notes: Median time closed issues spent in in-progress statuses, from first pickup to close.

## tasks.resolution_time — Time to resolution

- Source: task (task_metric_observations)
- Reads: resolution_days
- Formula: median(resolution_days)
- Shape: decimal, lower_is_better, unit d
- Notes: Median time from issue creation to close.

## tasks.pickup_time — Pickup time

- Source: task (task_metric_observations)
- Reads: pickup_days
- Formula: median(pickup_days)
- Shape: decimal, lower_is_better, unit d
- Notes: Median time from issue creation to first entering an in-progress status.

## tasks.flow_efficiency — Flow efficiency

- Source: task (task_metric_observations)
- Reads: flow_dev_seconds, flow_lead_seconds
- Formula: 100 * (flow_dev_seconds / flow_lead_seconds) -> x, clamped to <= 100
- Shape: percent, higher_is_better
- Notes: Time in active development as a share of total issue lifetime, across closed issues.

## tasks.reopen_rate — Reopen rate

- Source: task (task_metric_observations)
- Reads: reopened_within_14d, close_events
- Formula: 100 * (reopened_within_14d / close_events)
- Shape: percent, lower_is_better
- Notes: Share of issue closes followed by a reopen within 14 days.

## tasks.due_date_compliance — Due date compliance

- Source: task (task_metric_observations)
- Reads: due_date_on_time, due_date_with_due
- Formula: 100 * (due_date_on_time / due_date_with_due)
- Shape: percent, higher_is_better
- Notes: Share of issues that had a due date and were closed on or before it.

## tasks.on_time_delivery — On-time delivery

- Source: task (task_metric_observations)
- Reads: due_date_on_time, tasks_closed
- Formula: 100 * (due_date_on_time / tasks_closed)
- Shape: percent, higher_is_better
- Notes: Share of all closed issues that were closed on or before their due date.

## tasks.avg_slip — Average slip

- Source: task (task_metric_observations)
- Reads: slip_days_total, late_count
- Formula: slip_days_total / late_count
- Shape: decimal, lower_is_better, unit d
- Notes: Average days past the due date for issues closed late.

## tasks.estimation_accuracy — Estimation accuracy

- Source: task (task_metric_observations)
- Reads: estimation_error_pct, estimation_samples
- Formula: estimation_error_pct / estimation_samples -> -1*x + 100, clamped to [0, 100]
- Shape: percent, higher_is_better
- Notes: 100 minus the average deviation between original estimates and time spent, over days whose estimated work stayed within twice the estimate. 100 means estimates matched reality; over- and under-estimation count equally.

## tasks.worklog_accuracy — Worklog accuracy

- Source: task (task_metric_observations)
- Reads: worklog_seconds, in_progress_seconds
- Formula: 100 * (worklog_seconds / in_progress_seconds) -> x, clamped to <= 100
- Shape: percent, higher_is_better
- Notes: Logged work time as a share of time issues spent in in-progress statuses.

## tasks.bugs_ratio — Bugs share of closed issues

- Source: task (task_metric_observations)
- Reads: bugs_fixed, tasks_closed
- Formula: 100 * (bugs_fixed / tasks_closed)
- Shape: percent, lower_is_better
- Notes: Bug-type issues as a share of all closed issues, issues of an undetermined type included in the denominator. A share, so it cannot exceed 100%.

## tasks.stale_in_progress — Stale in progress

- Source: task (task_metric_observations)
- Reads: stale_in_progress
- Formula: sum(stale_in_progress)
- Shape: integer, lower_is_better, unit issues
- Notes: Open issues with no status change in more than 14 days.

## wiki.pages_created — Pages created

- Source: wiki (wiki_metric_observations)
- Reads: pages_created
- Formula: sum(pages_created)
- Shape: integer, higher_is_better, unit pages
- Notes: Wiki pages the person created during the period, counted on the page's creation date.

## wiki.edits — Page edits

- Source: wiki (wiki_metric_observations)
- Reads: edits
- Formula: sum(edits)
- Shape: integer, higher_is_better, unit edits
- Notes: Logical wiki edits the person made during the period. Consecutive saves of the same page within a short window count as one edit, so autosaves do not inflate the number.

## wiki.pages_edited — Pages edited

- Source: wiki (wiki_metric_observations)
- Reads: pages_edited
- Formula: sum(pages_edited)
- Shape: integer, higher_is_better, unit pages
- Notes: Distinct wiki pages the person edited during the period, counted per day the page was touched.

## wiki.comments — Comments received

- Source: wiki (wiki_metric_observations)
- Reads: comments
- Formula: sum(comments)
- Shape: integer, higher_is_better, unit comments
- Notes: Comments and replies other people left on wiki pages the person authored — a signal of how much their documentation is read and discussed.
