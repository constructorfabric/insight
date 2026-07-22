use crate::domain::metric_definitions::definition::{
    MetricComputation, MetricDirection, MetricFormat, MetricInputRole, SourceKind, ValueTransform,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityType {
    Person,
}

impl EntityType {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Person => "person",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CohortKey {
    OrgUnit,
}

impl CohortKey {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::OrgUnit => "org_unit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeedComputation {
    Sum,
    Ratio { scale: f64 },
    Median,
    DistinctCount,
}

impl SeedComputation {
    pub fn computation(self) -> MetricComputation {
        match self {
            Self::Sum => MetricComputation::Sum,
            Self::Ratio { .. } => MetricComputation::Ratio,
            Self::Median => MetricComputation::Median,
            Self::DistinctCount => MetricComputation::DistinctCount,
        }
    }

    pub fn scale(self) -> Option<f64> {
        match self {
            Self::Sum | Self::Median | Self::DistinctCount => None,
            Self::Ratio { scale } => Some(scale),
        }
    }
}

pub struct SourceSeed {
    pub key: &'static str,
    pub kind: SourceKind,
    /// Managed-observation relation name; must satisfy
    /// `ObservationRelation::parse` (pinned by a registry test).
    pub source_ref: &'static str,
}

pub struct BuiltinSource {
    pub source: SourceSeed,
    pub measures: &'static [&'static str],
    pub dimensions: &'static [&'static str],
}

pub struct MetricSeed {
    pub metric_key: &'static str,
    pub source_key: &'static str,
    pub label: &'static str,
    pub description: Option<&'static str>,
    pub explanation: Option<&'static str>,
    pub unit: Option<&'static str>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    pub entity_type: EntityType,
    pub computation: SeedComputation,
    /// Post-aggregation shaping (affine + clamp) applied by the compiler to
    /// every computed value; None = identity.
    pub transform: Option<ValueTransform>,
    pub peer_cohort_key: Option<CohortKey>,
    pub inputs: &'static [InputSeed],
    pub dimensions: &'static [&'static str],
}

pub struct InputSeed {
    pub input_role: MetricInputRole,
    pub measure_key: &'static str,
}

pub const BUILTIN_SOURCES: &[BuiltinSource] = &[
    BuiltinSource {
        source: SourceSeed {
            key: "ai_usage",
            kind: SourceKind::ManagedObservation,
            source_ref: "ai_metric_observations",
        },
        measures: &[
            "accepted_lines",
            "removed_lines",
            "active_day",
            "cost_usd",
            "accepted_edit_actions",
            "tool_use_offered",
            "assistant_messages",
            "assistant_actions",
            "dev_conversations",
            "chat_assistant_conversations",
        ],
        dimensions: &["tool", "surface"],
    },
    BuiltinSource {
        source: SourceSeed {
            key: "git",
            kind: SourceKind::ManagedObservation,
            source_ref: "git_metric_observations",
        },
        measures: &[
            "commit_count",
            "commit_day",
            "commit_change_size",
            "code_lines_added",
            "lines_added",
            "lines_removed",
            "pr_created",
            "pr_created_merged",
            "pr_merged",
            "pr_cycle_hours",
            "pr_change_size",
        ],
        dimensions: &[
            "category",
            "change_type",
            "destination_branch",
            "file_extension",
            "project",
            "repository",
            "source",
        ],
    },
    BuiltinSource {
        source: SourceSeed {
            key: "collab",
            kind: SourceKind::ManagedObservation,
            source_ref: "collab_metric_observations",
        },
        measures: &[
            "total_chat_messages",
            "channel_posts",
            "direct_and_group_messages",
            "meeting_hours",
            "meetings_attended",
            "meetings_organized",
            "adhoc_meetings_attended",
            "scheduled_meetings_attended",
            "emails_sent",
            "emails_received",
            "emails_read",
            "files_engaged",
            "files_shared_internal",
            "files_shared_external",
            "files_shared",
            "active_day",
            "active_modality",
            "chat_active_day",
            "meeting_free_day",
            "focus_hours",
            "working_hours",
        ],
        dimensions: &["tool", "scope"],
    },
    BuiltinSource {
        source: SourceSeed {
            key: "task",
            kind: SourceKind::ManagedObservation,
            source_ref: "task_metric_observations",
        },
        measures: &[
            "tasks_closed",
            "bugs_fixed",
            "due_date_on_time",
            "due_date_with_due",
            "slip_days_total",
            "late_count",
            "estimation_error_pct",
            "estimation_samples",
            "flow_dev_seconds",
            "flow_lead_seconds",
            "close_events",
            "reopened_within_14d",
            "worklog_seconds",
            "in_progress_seconds",
            "stale_in_progress",
            "dev_time_hours",
            "resolution_days",
            "pickup_days",
        ],
        dimensions: &[],
    },
    BuiltinSource {
        source: SourceSeed {
            key: "wiki",
            kind: SourceKind::ManagedObservation,
            source_ref: "wiki_metric_observations",
        },
        measures: &["pages_created", "edits", "pages_edited", "comments"],
        dimensions: &[],
    },
];

pub const BUILTIN_METRICS: &[MetricSeed] = &[
    MetricSeed {
        metric_key: "ai.accepted_lines",
        source_key: "ai_usage",
        label: "AI-added lines",
        description: Some("Accepted added coding output"),
        explanation: Some("Accepted AI-generated added lines across coding AI tools."),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "accepted_lines",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.removed_lines",
        source_key: "ai_usage",
        label: "AI-removed lines",
        description: Some("Accepted deleted coding output"),
        explanation: Some("Accepted AI-generated removed lines across coding AI tools."),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "removed_lines",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.active_days",
        source_key: "ai_usage",
        label: "AI active days",
        description: Some("Days with any AI activity across dev and assistant tools"),
        explanation: Some(
            "Distinct days with person-attributed AI activity across dev and assistant tools.",
        ),
        unit: Some("days"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "active_day",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "ai.cost",
        source_key: "ai_usage",
        label: "AI cost",
        description: Some("Reported AI spend across dev and assistant tools"),
        explanation: Some(
            "Person-attributed AI spend across dev and assistant tools, where the connector reports cost.",
        ),
        unit: None,
        format: MetricFormat::Currency,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "cost_usd",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.accepted_edit_actions",
        source_key: "ai_usage",
        label: "Accepted AI edits",
        description: Some("Accepted tool or edit suggestions"),
        explanation: Some("Accepted AI edit or tool suggestions across supported coding AI tools."),
        unit: Some("actions"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "accepted_edit_actions",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.tool_acceptance_rate",
        source_key: "ai_usage",
        label: "AI tool acceptance",
        description: Some("Accepted divided by offered AI edits"),
        explanation: Some("Accepted AI edit or tool suggestions divided by offered suggestions."),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "accepted_edit_actions",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "tool_use_offered",
            },
        ],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.assistant_messages",
        source_key: "ai_usage",
        label: "AI assistant messages",
        description: Some("Assistant messages"),
        explanation: Some(
            "Person-attributed assistant messages from supported AI assistant tools.",
        ),
        unit: Some("messages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "assistant_messages",
        }],
        dimensions: &["tool", "surface"],
    },
    MetricSeed {
        metric_key: "ai.assistant_actions",
        source_key: "ai_usage",
        label: "AI assistant actions",
        description: Some("Assistant actions"),
        explanation: Some("Person-attributed assistant actions from supported AI assistant tools."),
        unit: Some("actions"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "assistant_actions",
        }],
        dimensions: &["tool", "surface"],
    },
    MetricSeed {
        metric_key: "ai.dev_conversations",
        source_key: "ai_usage",
        label: "AI dev conversations",
        description: Some("Coding tool conversations where the source reports them"),
        explanation: Some(
            "Person-attributed coding conversations from dev tools that report them.",
        ),
        unit: Some("conversations"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "dev_conversations",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "ai.chat_assistant_conversations",
        source_key: "ai_usage",
        label: "AI chat conversations",
        description: Some("Chat assistant conversations"),
        explanation: Some(
            "Person-attributed chat assistant conversations from supported AI chat tools.",
        ),
        unit: Some("conversations"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "chat_assistant_conversations",
        }],
        dimensions: &["tool", "surface"],
    },
    MetricSeed {
        metric_key: "git.commits",
        source_key: "git",
        label: "Commits",
        description: Some("Authored commits"),
        explanation: Some(
            "Distinct authored commits across connected git sources, excluding merge commits.",
        ),
        unit: Some("commits"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "commit_count",
        }],
        dimensions: &["project", "repository", "source"],
    },
    MetricSeed {
        metric_key: "git.code_lines",
        source_key: "git",
        label: "Code lines added",
        description: Some("Lines added to code files"),
        explanation: Some(
            "Lines added to files classified as code — tests, configuration, and documentation excluded.",
        ),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "code_lines_added",
        }],
        dimensions: &[
            "change_type",
            "file_extension",
            "project",
            "repository",
            "source",
        ],
    },
    MetricSeed {
        metric_key: "git.lines_added",
        source_key: "git",
        label: "Lines added",
        description: Some("All lines added, by file category"),
        explanation: Some(
            "Lines added across all files, split by file category: code, tests, configuration, documentation.",
        ),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "lines_added",
        }],
        dimensions: &[
            "category",
            "change_type",
            "file_extension",
            "project",
            "repository",
            "source",
        ],
    },
    MetricSeed {
        metric_key: "git.lines_removed",
        source_key: "git",
        label: "Lines removed",
        description: Some("All lines removed, by file category"),
        explanation: Some(
            "Lines removed across all reported file changes, with file-category, repository, and source breakdowns available.",
        ),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "lines_removed",
        }],
        dimensions: &[
            "category",
            "change_type",
            "file_extension",
            "project",
            "repository",
            "source",
        ],
    },
    MetricSeed {
        metric_key: "git.prs_created",
        source_key: "git",
        label: "Pull requests created",
        description: Some("Authored pull requests"),
        explanation: Some("Pull requests opened, dated by creation."),
        unit: Some("PRs"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_created",
        }],
        dimensions: &["destination_branch", "project", "repository", "source"],
    },
    MetricSeed {
        metric_key: "git.prs_merged",
        source_key: "git",
        label: "Pull requests merged",
        description: Some("Authored pull requests merged"),
        explanation: Some("Authored pull requests that merged, dated by the merge."),
        unit: Some("PRs"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_merged",
        }],
        dimensions: &["destination_branch", "project", "repository", "source"],
    },
    MetricSeed {
        metric_key: "git.merge_rate",
        source_key: "git",
        label: "PR merge rate",
        description: Some("Share of created pull requests that merged"),
        explanation: Some(
            "Of the pull requests created in the period, the share that have merged. Requests opened near the end of the period may not have merged yet, which lowers the rate at period edges.",
        ),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "pr_created_merged",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "pr_created",
            },
        ],
        dimensions: &["destination_branch", "project", "repository", "source"],
    },
    MetricSeed {
        metric_key: "git.commits_per_active_day",
        source_key: "git",
        label: "Commits per active day",
        description: Some("Commit cadence on days with commits"),
        explanation: Some("Commits divided by the number of days with at least one commit."),
        unit: None,
        format: MetricFormat::Decimal,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 1.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "commit_count",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "commit_day",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "git.commit_size",
        source_key: "git",
        label: "Commit size",
        description: Some("Typical diff size per commit"),
        explanation: Some(
            "Median diff size of authored commits (lines added plus removed). Smaller commits are easier to review.",
        ),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Median,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "commit_change_size",
        }],
        dimensions: &["project", "repository", "source"],
    },
    MetricSeed {
        metric_key: "git.pr_size",
        source_key: "git",
        label: "PR size",
        description: Some("Typical diff size per pull request"),
        explanation: Some(
            "Median diff size of authored pull requests (lines added plus removed). Smaller requests are easier to review. Sources that do not report line counts contribute no values.",
        ),
        unit: Some("lines"),
        format: MetricFormat::Integer,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Median,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_change_size",
        }],
        dimensions: &["destination_branch", "project", "repository", "source"],
    },
    MetricSeed {
        metric_key: "git.pr_cycle_time_h",
        source_key: "git",
        label: "PR cycle time",
        description: Some("Typical hours from open to merge"),
        explanation: Some(
            "Median hours from opening a pull request to merging it, over requests merged in the period.",
        ),
        unit: Some("h"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Median,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_cycle_hours",
        }],
        dimensions: &["destination_branch", "project", "repository", "source"],
    },
    // ─────────────────────────── collaboration ───────────────────────────
    MetricSeed {
        metric_key: "collab.messages_sent",
        source_key: "collab",
        label: "Messages Sent",
        description: Some("Chat messages sent"),
        explanation: Some(
            "Chat messages a person sent across messaging tools. Counts are not directly comparable between tools: Slack includes thread replies, and Microsoft 365 combines private-chat and channel messages.",
        ),
        unit: Some("messages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "total_chat_messages",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.channel_posts",
        source_key: "collab",
        label: "Channel Posts",
        description: Some("Messages posted to shared channels, including replies"),
        explanation: Some(
            "Channel posts plus thread replies across messaging tools. Tools that report posts and replies separately are folded so counts stay comparable.",
        ),
        unit: Some("messages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "channel_posts",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.dm_ratio",
        source_key: "collab",
        label: "DM Ratio",
        description: Some("Share of messages sent in direct or group chats"),
        explanation: Some(
            "Direct and group-chat messages divided by all chat messages. A lower ratio means more communication happens in open channels. Tools that do not distinguish message types report no value.",
        ),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "direct_and_group_messages",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "total_chat_messages",
            },
        ],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.msgs_per_active_day",
        source_key: "collab",
        label: "Messages per Active Day",
        description: Some("Chat messages divided by chat-active days"),
        explanation: Some(
            "Chat messages sent divided by days with chat messages. Each tool's active days count separately.",
        ),
        unit: Some("messages/day"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 1.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "total_chat_messages",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "chat_active_day",
            },
        ],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.active_days",
        source_key: "collab",
        label: "Active Days",
        description: Some("Days with collaboration activity"),
        explanation: Some(
            "Distinct days on which a person took a deliberate collaboration action — sending a message, sending email, engaging or sharing a file, or attending a meeting. Passive activity such as receiving or reading email is excluded.",
        ),
        unit: Some("days"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::DistinctCount,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "active_day",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.emails_sent",
        source_key: "collab",
        label: "Emails Sent",
        description: Some("Emails sent"),
        explanation: Some("Emails a person sent."),
        unit: Some("emails"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "emails_sent",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.emails_received",
        source_key: "collab",
        label: "Emails Received",
        description: Some("Emails received"),
        explanation: Some("Emails a person received."),
        unit: Some("emails"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "emails_received",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.emails_read",
        source_key: "collab",
        label: "Emails Read",
        description: Some("Emails read"),
        explanation: Some("Emails a person read."),
        unit: Some("emails"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "emails_read",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.files_engaged",
        source_key: "collab",
        label: "Files Engaged",
        description: Some("Files viewed or edited"),
        explanation: Some("Files a person viewed or edited."),
        unit: Some("files"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "files_engaged",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.files_shared_internal",
        source_key: "collab",
        label: "Files Shared (Internal)",
        description: Some("Files shared inside the organization"),
        explanation: Some("Files a person shared with people inside the organization."),
        unit: Some("files"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "files_shared_internal",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.files_shared_external",
        source_key: "collab",
        label: "Files Shared (External)",
        description: Some("Files shared outside the organization"),
        explanation: Some("Files a person shared with people outside the organization."),
        unit: Some("files"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "files_shared_external",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.files_shared",
        source_key: "collab",
        label: "Files Shared",
        description: Some("Files shared with any recipient"),
        explanation: Some(
            "Files a person shared with recipients inside or outside the organization.",
        ),
        unit: Some("files"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "files_shared",
        }],
        dimensions: &["scope"],
    },
    MetricSeed {
        metric_key: "collab.meeting_hours",
        source_key: "collab",
        label: "Meeting Hours",
        description: Some("Hours spent in meetings"),
        explanation: Some(
            "Hours spent in meetings, taking the longest active modality (audio, video, or screen share) per meeting. Zoom reports modality durations as full-session estimates, so its figures may run higher than Microsoft Teams.",
        ),
        unit: Some("h"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "meeting_hours",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.meetings_count",
        source_key: "collab",
        label: "Meetings Attended",
        description: Some("Distinct meetings attended"),
        explanation: Some("Distinct meetings a person attended across meeting tools."),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "meetings_attended",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.meeting_free_days",
        source_key: "collab",
        label: "Meeting-Free Days",
        description: Some("Active days with no meeting time"),
        explanation: Some(
            "Days on which a person was actively collaborating but spent no time in meetings — a proxy for uninterrupted working days.",
        ),
        unit: Some("days"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "meeting_free_day",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "collab.focus_time_pct",
        source_key: "collab",
        label: "Focus Time",
        description: Some("Share of the workday outside meetings"),
        explanation: Some(
            "Share of the workday not spent in meetings: meeting-free hours divided by scheduled working hours. Scheduled hours default to a nominal eight-hour day where an HR source does not provide them.",
        ),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "focus_hours",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "working_hours",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "collab.breadth",
        source_key: "collab",
        label: "Collaboration Breadth",
        description: Some("Distinct collaboration modalities used"),
        explanation: Some(
            "Distinct collaboration modalities — chat, meetings, email, documents — a person was deliberately active in during the period.",
        ),
        unit: Some("modalities"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::DistinctCount,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "active_modality",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "collab.meetings_organized",
        source_key: "collab",
        label: "Meetings Organized",
        description: Some("Meetings organized"),
        explanation: Some(
            "Meetings a person organized. Reported only by tools that expose organizer counts.",
        ),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "meetings_organized",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.adhoc_meetings",
        source_key: "collab",
        label: "Ad-hoc Meetings",
        description: Some("Unscheduled meetings attended"),
        explanation: Some(
            "Unscheduled meetings a person attended. Reported only by tools that distinguish ad-hoc from scheduled meetings.",
        ),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "adhoc_meetings_attended",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.scheduled_meetings",
        source_key: "collab",
        label: "Scheduled Meetings",
        description: Some("Scheduled meetings attended"),
        explanation: Some(
            "Scheduled meetings a person attended. Reported only by tools that distinguish ad-hoc from scheduled meetings.",
        ),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "scheduled_meetings_attended",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "tasks.closed",
        source_key: "task",
        label: "Tasks closed",
        description: Some("Tasks moved to a closed status"),
        explanation: Some("Tasks a person moved into a closed status during the period."),
        unit: Some("tasks"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "tasks_closed",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.bugs_fixed",
        source_key: "task",
        label: "Bugs fixed",
        description: Some("Bug-type tasks closed"),
        explanation: Some("Bug-type tasks a person closed during the period."),
        unit: Some("tasks"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "bugs_fixed",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.dev_time",
        source_key: "task",
        label: "Development time",
        description: Some("Time a task spends in active development"),
        explanation: Some(
            "Median time closed tasks spent in in-progress statuses, from first pickup to close.",
        ),
        unit: Some("h"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Median,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "dev_time_hours",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.resolution_time",
        source_key: "task",
        label: "Time to resolution",
        description: Some("Task lifetime from creation to close"),
        explanation: Some("Median time from task creation to close."),
        unit: Some("d"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Median,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "resolution_days",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.pickup_time",
        source_key: "task",
        label: "Pickup time",
        description: Some("Wait before work starts on a task"),
        explanation: Some(
            "Median time from task creation to first entering an in-progress status.",
        ),
        unit: Some("d"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Median,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pickup_days",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.flow_efficiency",
        source_key: "task",
        label: "Flow efficiency",
        description: Some("Active development share of task lifetime"),
        explanation: Some(
            "Time in active development as a share of total task lifetime, across closed tasks.",
        ),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: Some(ValueTransform {
            multiplier: None,
            offset: None,
            clamp_min: None,
            clamp_max: Some(100.0),
        }),
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "flow_dev_seconds",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "flow_lead_seconds",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.reopen_rate",
        source_key: "task",
        label: "Reopen rate",
        description: Some("Closed tasks reopened shortly after"),
        explanation: Some("Share of task closes followed by a reopen within 14 days."),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "reopened_within_14d",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "close_events",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.due_date_compliance",
        source_key: "task",
        label: "Due date compliance",
        description: Some("On-time share of tasks with a due date"),
        explanation: Some("Share of tasks that had a due date and were closed on or before it."),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "due_date_on_time",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "due_date_with_due",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.on_time_delivery",
        source_key: "task",
        label: "On-time delivery",
        description: Some("On-time share of all closed tasks"),
        explanation: Some(
            "Share of all closed tasks that were closed on or before their due date.",
        ),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "due_date_on_time",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "tasks_closed",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.avg_slip",
        source_key: "task",
        label: "Average slip",
        description: Some("How late overdue tasks close"),
        explanation: Some("Average days past the due date for tasks closed late."),
        unit: Some("d"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 1.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "slip_days_total",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "late_count",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.estimation_accuracy",
        source_key: "task",
        label: "Estimation accuracy",
        description: Some("How close estimates land to time spent"),
        explanation: Some(
            "100 minus the average deviation between original estimates and time spent, over days whose estimated work stayed within twice the estimate. 100 means estimates matched reality; over- and under-estimation count equally.",
        ),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 1.0 },
        transform: Some(ValueTransform {
            multiplier: Some(-1.0),
            offset: Some(100.0),
            clamp_min: Some(0.0),
            clamp_max: Some(100.0),
        }),
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "estimation_error_pct",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "estimation_samples",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.worklog_accuracy",
        source_key: "task",
        label: "Worklog accuracy",
        description: Some("Logged time versus tracked development"),
        explanation: Some(
            "Logged work time as a share of time tasks spent in in-progress statuses.",
        ),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: Some(ValueTransform {
            multiplier: None,
            offset: None,
            clamp_min: None,
            clamp_max: Some(100.0),
        }),
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "worklog_seconds",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "in_progress_seconds",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.bugs_ratio",
        source_key: "task",
        label: "Bug ratio",
        description: Some("Bugs as a share of closed tasks"),
        explanation: Some("Bug-type tasks as a share of all closed tasks."),
        unit: None,
        format: MetricFormat::Percent,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[
            InputSeed {
                input_role: MetricInputRole::Numerator,
                measure_key: "bugs_fixed",
            },
            InputSeed {
                input_role: MetricInputRole::Denominator,
                measure_key: "tasks_closed",
            },
        ],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "tasks.stale_in_progress",
        source_key: "task",
        label: "Stale in progress",
        description: Some("Open tasks idle for over two weeks"),
        explanation: Some("Open tasks with no status change in more than 14 days."),
        unit: Some("tasks"),
        format: MetricFormat::Integer,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "stale_in_progress",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "wiki.pages_created",
        source_key: "wiki",
        label: "Pages created",
        description: Some("Wiki pages authored"),
        explanation: Some(
            "Wiki pages the person created during the period, counted on the \
             page's creation date.",
        ),
        unit: Some("pages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pages_created",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "wiki.edits",
        source_key: "wiki",
        label: "Page edits",
        description: Some("Wiki edit sessions"),
        explanation: Some(
            "Logical wiki edits the person made during the period. Consecutive \
             saves of the same page within a short window count as one edit, so \
             autosaves do not inflate the number.",
        ),
        unit: Some("edits"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "edits",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "wiki.pages_edited",
        source_key: "wiki",
        label: "Pages edited",
        description: Some("Distinct wiki pages edited"),
        explanation: Some(
            "Distinct wiki pages the person edited during the period, counted \
             per day the page was touched.",
        ),
        unit: Some("pages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pages_edited",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "wiki.comments",
        source_key: "wiki",
        label: "Comments received",
        description: Some("Comments on the person's wiki pages"),
        explanation: Some(
            "Comments and replies other people left on wiki pages the person \
             authored — a signal of how much their documentation is read and \
             discussed.",
        ),
        unit: Some("comments"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        transform: None,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "comments",
        }],
        dimensions: &[],
    },
];

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use super::*;

    fn is_snake_case(value: &str) -> bool {
        !value.is_empty()
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
    }

    fn is_metric_key(value: &str) -> bool {
        let parts = value.split('.').collect::<Vec<_>>();
        parts.len() == 2 && parts.iter().all(|part| is_snake_case(part))
    }

    #[test]
    fn source_keys_are_unique_and_shaped() {
        let mut seen = BTreeSet::new();
        for builtin_source in BUILTIN_SOURCES {
            assert!(is_snake_case(builtin_source.source.key));
            assert!(seen.insert(builtin_source.source.key));
        }
    }

    #[test]
    fn source_refs_parse_as_observation_relations() {
        use crate::domain::metric_definitions::definition::ObservationRelation;
        for builtin_source in BUILTIN_SOURCES {
            assert!(
                ObservationRelation::parse(builtin_source.source.source_ref).is_some(),
                "builtin source {} declares an invalid observation relation {:?}",
                builtin_source.source.key,
                builtin_source.source.source_ref,
            );
        }
    }

    #[test]
    fn measure_and_dimension_keys_are_unique_per_source() {
        for builtin_source in BUILTIN_SOURCES {
            let mut measures = BTreeSet::new();
            for measure_key in builtin_source.measures {
                assert!(is_snake_case(measure_key));
                assert!(measures.insert(*measure_key));
            }
            let mut dimensions = BTreeSet::new();
            for dimension_key in builtin_source.dimensions {
                assert!(is_snake_case(dimension_key));
                assert!(dimensions.insert(*dimension_key));
            }
        }
    }

    #[test]
    fn metric_keys_are_unique_and_shaped() {
        let mut seen = BTreeSet::new();
        for metric in BUILTIN_METRICS {
            assert!(is_metric_key(metric.metric_key), "{}", metric.metric_key);
            assert!(seen.insert(metric.metric_key));
        }
    }

    #[test]
    fn metric_inputs_reference_declared_measures() {
        let measures_by_source: HashMap<&str, BTreeSet<&str>> = BUILTIN_SOURCES
            .iter()
            .map(|builtin_source| {
                (
                    builtin_source.source.key,
                    builtin_source.measures.iter().copied().collect(),
                )
            })
            .collect();

        for metric in BUILTIN_METRICS {
            let measures = measures_by_source
                .get(metric.source_key)
                .unwrap_or_else(|| panic!("unknown source for {}", metric.metric_key));
            assert!(!metric.inputs.is_empty(), "{}", metric.metric_key);
            for input in metric.inputs {
                assert!(
                    measures.contains(input.measure_key),
                    "{} references undeclared measure {}",
                    metric.metric_key,
                    input.measure_key
                );
            }
        }
    }

    #[test]
    fn metric_dimensions_reference_declared_source_dimensions() {
        let dimensions_by_source: HashMap<&str, BTreeSet<&str>> = BUILTIN_SOURCES
            .iter()
            .map(|builtin_source| {
                (
                    builtin_source.source.key,
                    builtin_source.dimensions.iter().copied().collect(),
                )
            })
            .collect();

        for metric in BUILTIN_METRICS {
            let Some(dimensions) = dimensions_by_source.get(metric.source_key) else {
                panic!("unknown source for {}", metric.metric_key);
            };
            for dimension in metric.dimensions {
                assert!(
                    dimensions.contains(dimension),
                    "{} references undeclared dimension {dimension}",
                    metric.metric_key
                );
            }
        }
    }

    #[test]
    fn ratio_metrics_have_numerator_and_denominator_roles() {
        for metric in BUILTIN_METRICS {
            let SeedComputation::Ratio { .. } = metric.computation else {
                continue;
            };
            let has_role = |role| metric.inputs.iter().any(|input| input.input_role == role);
            assert!(
                has_role(MetricInputRole::Numerator),
                "{}",
                metric.metric_key
            );
            assert!(
                has_role(MetricInputRole::Denominator),
                "{}",
                metric.metric_key
            );
        }
    }

    #[test]
    fn median_metrics_have_single_value_role() {
        for metric in BUILTIN_METRICS {
            if metric.computation != SeedComputation::Median {
                continue;
            }
            assert_eq!(metric.inputs.len(), 1, "{}", metric.metric_key);
            assert_eq!(
                metric.inputs[0].input_role,
                MetricInputRole::Value,
                "{}",
                metric.metric_key
            );
        }
    }

    #[test]
    fn distinct_count_metrics_have_single_value_role() {
        for metric in BUILTIN_METRICS {
            if metric.computation != SeedComputation::DistinctCount {
                continue;
            }
            assert_eq!(metric.inputs.len(), 1, "{}", metric.metric_key);
            assert_eq!(
                metric.inputs[0].input_role,
                MetricInputRole::Value,
                "{}",
                metric.metric_key
            );
        }
    }

    // Percent and currency formats are presentation-complete: the FE's
    // formatMetricValue/metricDisplayUnit always render "%" or a currency
    // symbol from `format` alone and never consult `unit` for these two
    // formats. A unit string here is therefore dead config that only invites
    // drift (e.g. "percent" vs "%" for the same format) — keep it None.
    #[test]
    fn presentation_complete_formats_carry_no_unit() {
        for metric in BUILTIN_METRICS {
            if !matches!(
                metric.format,
                MetricFormat::Percent | MetricFormat::Currency
            ) {
                continue;
            }
            assert!(
                metric.unit.is_none(),
                "{} has format {:?}, which renders without consulting unit; unit must be None",
                metric.metric_key,
                metric.format
            );
        }
    }
}
