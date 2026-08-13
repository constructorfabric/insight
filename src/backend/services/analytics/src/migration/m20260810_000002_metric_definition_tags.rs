//! Adds free-form tags to metric definitions. Tags are cross-cutting labels a
//! surface can filter or search by (e.g. `time`, `calendar`) — many per
//! metric, unlike the singular grouping `subject`. Tags are plain slugs, not
//! bound to a source the way dimensions are, so this table carries the label
//! directly rather than referencing `metric_source_dimensions`.

use sea_orm_migration::prelude::*;

pub const REQUIRED_TAG_CHECKS: &[&str] = &[
    "chk_metric_definition_tags_tag_shape",
    "chk_metric_definition_tags_display_order_nonnegative",
];

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS metric_definition_tags (
    id BINARY(16) NOT NULL PRIMARY KEY,
    metric_definition_id BINARY(16) NOT NULL,
    tag VARCHAR(64) NOT NULL,
    display_order INT NOT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_metric_definition_tags_tag (metric_definition_id, tag),
    CONSTRAINT fk_metric_definition_tags_definition FOREIGN KEY (metric_definition_id) REFERENCES metric_definitions(id) ON DELETE CASCADE,
    CONSTRAINT chk_metric_definition_tags_tag_shape CHECK (tag REGEXP BINARY '^[a-z][a-z0-9_]*$'),
    CONSTRAINT chk_metric_definition_tags_display_order_nonnegative CHECK (display_order >= 0)
)";

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(CREATE_TABLE)
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS metric_definition_tags")
            .await?;
        Ok(())
    }
}
