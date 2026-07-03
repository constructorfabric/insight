use sea_orm::{ConnectionTrait, Statement, Value};
use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

const AI_PERSONAL_COUNTERS_HEX: &str = "00000000000000000001000000000052";

struct SeedRow {
    metric_key: &'static str,
    label: &'static str,
    sublabel: Option<&'static str>,
    description: Option<&'static str>,
    unit: Option<&'static str>,
    format: Option<&'static str>,
    higher_is_better: bool,
    good: f64,
    warn: f64,
}

const SEEDS: &[SeedRow] = &[
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_accepted_lines",
        label: "AI-added lines",
        sublabel: Some("Accepted added coding output"),
        description: Some("Accepted AI-generated added lines across coding AI tools."),
        unit: Some("lines"),
        format: Some("integer"),
        higher_is_better: true,
        good: 1000.0,
        warn: 100.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_active_days",
        label: "AI active days",
        sublabel: Some("Days with coding AI activity"),
        description: Some("Distinct days with person-attributed coding AI activity."),
        unit: Some("days"),
        format: None,
        higher_is_better: true,
        good: 10.0,
        warn: 3.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_removed_lines",
        label: "AI-removed lines",
        sublabel: Some("Accepted deleted coding output"),
        description: Some("Accepted AI-generated removed lines across coding AI tools."),
        unit: Some("lines"),
        format: Some("integer"),
        higher_is_better: true,
        good: 0.0,
        warn: 1000.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_cost",
        label: "Reported AI cost",
        sublabel: Some("Metered tool spend"),
        description: Some("Person-attributed AI cost where the connector reports cost."),
        unit: Some("$"),
        format: Some("currency"),
        higher_is_better: false,
        good: 0.0,
        warn: 50.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_accepted_edit_actions",
        label: "Accepted AI edits",
        sublabel: Some("Accepted tool/edit suggestions"),
        description: Some("Accepted AI edit or tool suggestions across supported coding AI tools."),
        unit: Some("actions"),
        format: Some("integer"),
        higher_is_better: true,
        good: 50.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_tool_acceptance_rate",
        label: "AI tool acceptance",
        sublabel: Some("Accepted ÷ offered AI edits"),
        description: Some("Accepted AI edit/tool suggestions divided by offered suggestions."),
        unit: Some("%"),
        format: Some("percent"),
        higher_is_better: true,
        good: 50.0,
        warn: 20.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_assistant_messages",
        label: "AI assistant messages",
        sublabel: Some("Chat and assistant messages"),
        description: Some(
            "Person-attributed assistant messages from supported AI assistant tools.",
        ),
        unit: Some("messages"),
        format: Some("integer"),
        higher_is_better: true,
        good: 50.0,
        warn: 10.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_assistant_actions",
        label: "AI assistant actions",
        sublabel: Some("Assistant/cowork actions"),
        description: Some("Person-attributed assistant actions from supported AI assistant tools."),
        unit: Some("actions"),
        format: Some("integer"),
        higher_is_better: true,
        good: 10.0,
        warn: 1.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_dev_agent_conversations",
        label: "AI dev conversations",
        sublabel: Some("Claude Code sessions and Codex threads"),
        description: Some(
            "Person-attributed coding-agent conversations from supported AI dev tools.",
        ),
        unit: Some("conversations"),
        format: Some("integer"),
        higher_is_better: true,
        good: 20.0,
        warn: 5.0,
    },
    SeedRow {
        metric_key: "ai_person_counter_daily.ai_chat_assistant_conversations",
        label: "AI chat conversations",
        sublabel: Some("Chat assistant threads"),
        description: Some(
            "Person-attributed chat assistant conversations from supported AI chat tools.",
        ),
        unit: Some("conversations"),
        format: Some("integer"),
        higher_is_better: true,
        good: 20.0,
        warn: 5.0,
    },
];

const INSERT_CATALOG_SQL: &str = "\
    INSERT INTO metric_catalog \
        (id, tenant_id, metric_key, label, sublabel, description, unit, format, \
         higher_is_better, is_member_scale, source_tags, is_enabled) \
    VALUES (?, NULL, ?, ?, ?, ?, ?, ?, ?, FALSE, ?, TRUE) \
    ON DUPLICATE KEY UPDATE \
        label = VALUES(label), \
        sublabel = VALUES(sublabel), \
        description = VALUES(description), \
        unit = VALUES(unit), \
        format = VALUES(format), \
        higher_is_better = VALUES(higher_is_better), \
        is_member_scale = VALUES(is_member_scale), \
        source_tags = VALUES(source_tags), \
        is_enabled = VALUES(is_enabled)";

const INSERT_THRESHOLD_SQL: &str = "\
    INSERT INTO metric_threshold \
        (id, tenant_id, metric_key, scope, role_slug, team_id, good, warn, is_locked) \
    VALUES (?, NULL, ?, 'product-default', '', '', ?, ?, FALSE) \
    ON DUPLICATE KEY UPDATE \
        good = VALUES(good), \
        warn = VALUES(warn)";

const INSERT_LINK_SQL: &str = "\
    INSERT IGNORE INTO metric_query_catalog \
        (id, metrics_id, metric_catalog_id) \
    SELECT UNHEX(REPLACE(UUID(),'-','')), UNHEX(?), c.id \
    FROM metric_catalog c \
    WHERE c.metric_key = ? AND c.tenant_id IS NULL";

fn nullable_str_value(v: Option<&str>) -> Value {
    match v {
        Some(s) => Value::from(s),
        None => Value::String(None),
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();
        let backend = manager.get_database_backend();

        for row in SEEDS {
            let catalog_id = Uuid::now_v7();
            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_CATALOG_SQL,
                [
                    Value::Bytes(Some(Box::new(catalog_id.as_bytes().to_vec()))),
                    Value::from(row.metric_key),
                    Value::from(row.label),
                    nullable_str_value(row.sublabel),
                    nullable_str_value(row.description),
                    nullable_str_value(row.unit),
                    nullable_str_value(row.format),
                    Value::from(row.higher_is_better),
                    Value::from("[]"),
                ],
            ))
            .await?;

            let threshold_id = Uuid::now_v7();
            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_THRESHOLD_SQL,
                [
                    Value::Bytes(Some(Box::new(threshold_id.as_bytes().to_vec()))),
                    Value::from(row.metric_key),
                    Value::from(row.good),
                    Value::from(row.warn),
                ],
            ))
            .await?;

            conn.execute(Statement::from_sql_and_values(
                backend,
                INSERT_LINK_SQL,
                [
                    Value::from(AI_PERSONAL_COUNTERS_HEX),
                    Value::from(row.metric_key),
                ],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom("we have only forward migrations".to_owned()))
    }
}
