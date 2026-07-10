use crate::domain::metric_definitions::definition::{
    MetricComputation, MetricDirection, MetricFormat, MetricInputRole, SourceKind,
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
            "pr_created",
            "pr_created_merged",
            "pr_merged",
            "pr_cycle_hours",
            "pr_change_size",
        ],
        dimensions: &["source", "category"],
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
            "active_day",
            "active_tool",
            "chat_active_day",
            "meeting_free_day",
            "focus_hours",
            "working_hours",
        ],
        dimensions: &["tool"],
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
        unit: Some("percent"),
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "commit_count",
        }],
        dimensions: &["source"],
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "code_lines_added",
        }],
        dimensions: &["source"],
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "lines_added",
        }],
        dimensions: &["category", "source"],
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_created",
        }],
        dimensions: &["source"],
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_merged",
        }],
        dimensions: &["source"],
    },
    MetricSeed {
        metric_key: "git.merge_rate",
        source_key: "git",
        label: "PR merge rate",
        description: Some("Share of created pull requests that merged"),
        explanation: Some(
            "Of the pull requests created in the period, the share that have merged. Requests opened near the end of the period may not have merged yet, which lowers the rate at period edges.",
        ),
        unit: Some("percent"),
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
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
        dimensions: &[],
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "commit_change_size",
        }],
        dimensions: &["source"],
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_change_size",
        }],
        dimensions: &["source"],
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "pr_cycle_hours",
        }],
        dimensions: &["source"],
    },
    // ─────────────────────────── collaboration ───────────────────────────
    MetricSeed {
        metric_key: "collab.messages_sent",
        source_key: "collab",
        label: "Messages Sent",
        description: Some("Chat messages sent"),
        explanation: Some(
            "Chat messages a person sent, per messaging tool. Counts are not directly comparable across tools: Slack includes thread replies, and Microsoft 365 combines private-chat and channel messages.",
        ),
        unit: Some("messages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
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
        description: Some("Messages posted to shared channels"),
        explanation: Some("Messages a person posted to shared channels, per messaging tool."),
        unit: Some("messages"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
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
            "Direct and group-chat messages divided by all chat messages, per tool. A lower ratio means more communication happens in open channels. Tools that do not distinguish message types report no value.",
        ),
        unit: Some("%"),
        format: MetricFormat::Percent,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
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
            "Chat messages sent divided by the number of days with chat messages, per tool. Across tools, each tool's active days count separately.",
        ),
        unit: Some("messages/day"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 1.0 },
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
            "Distinct days on which a person took a deliberate collaboration action — sending a message, sending email, engaging or sharing a file, or attending a meeting — per tool. Passive activity such as receiving or reading email is excluded.",
        ),
        unit: Some("days"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::DistinctCount,
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
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "files_shared_external",
        }],
        dimensions: &["tool"],
    },
    MetricSeed {
        metric_key: "collab.meeting_hours",
        source_key: "collab",
        label: "Meeting Hours",
        description: Some("Hours spent in meetings"),
        explanation: Some(
            "Hours spent in meetings, per tool, taking the longest active modality (audio, video, or screen share) per meeting. Zoom reports modality durations as full-session estimates, so its figures may run higher than Microsoft Teams.",
        ),
        unit: Some("h"),
        format: MetricFormat::Decimal,
        direction: MetricDirection::LowerIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
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
        explanation: Some("Distinct meetings a person attended, per tool."),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
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
        description: Some("Days with no meeting time"),
        explanation: Some(
            "Days on which a person had meeting records but no meeting time — a proxy for uninterrupted working days.",
        ),
        unit: Some("days"),
        format: MetricFormat::Integer,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
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
        unit: Some("%"),
        format: MetricFormat::Percent,
        direction: MetricDirection::HigherIsBetter,
        entity_type: EntityType::Person,
        computation: SeedComputation::Ratio { scale: 100.0 },
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
        description: Some("Distinct collaboration tools used"),
        explanation: Some(
            "Distinct collaboration tools on which a person took a deliberate action in the period.",
        ),
        unit: Some("tools"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::DistinctCount,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "active_tool",
        }],
        dimensions: &[],
    },
    MetricSeed {
        metric_key: "collab.meetings_organized",
        source_key: "collab",
        label: "Meetings Organized",
        description: Some("Meetings organized"),
        explanation: Some(
            "Meetings a person organized, per tool. Reported only where the tool exposes organizer counts.",
        ),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
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
            "Unscheduled meetings a person attended, per tool. Reported only where the tool distinguishes ad-hoc from scheduled meetings.",
        ),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
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
            "Scheduled meetings a person attended, per tool. Reported only where the tool distinguishes ad-hoc from scheduled meetings.",
        ),
        unit: Some("meetings"),
        format: MetricFormat::Integer,
        direction: MetricDirection::Neutral,
        entity_type: EntityType::Person,
        computation: SeedComputation::Sum,
        peer_cohort_key: Some(CohortKey::OrgUnit),
        inputs: &[InputSeed {
            input_role: MetricInputRole::Value,
            measure_key: "scheduled_meetings_attended",
        }],
        dimensions: &["tool"],
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
}
