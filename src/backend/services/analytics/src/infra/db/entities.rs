//! `SeaORM` entity definitions for `MariaDB` tables.

pub mod metrics {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "metrics")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub insight_tenant_id: Uuid,
        pub name: String,
        pub description: Option<String>,
        pub query_ref: String,
        pub is_enabled: bool,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod thresholds {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "thresholds")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub insight_tenant_id: Uuid,
        pub metric_id: Uuid,
        pub field_name: String,
        pub operator: String,
        #[sea_orm(column_type = "Decimal(Some((20, 6)))")]
        pub value: f64,
        pub level: String,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod metric_catalog {
    //! `metric_catalog` entity (Refs #519 schema, #521 validator reads/writes).
    //!
    //! The validator only reads/writes a narrow column set; the rest of the
    //! catalog row exists but is owned by other components (seed-migration for
    //! product columns, admin-crud for read joins). Typed access is exposed for
    //! that narrow set; the no-`updated_at` write path uses raw SQL with bound
    //! parameters from `domain::schema_validator::repository` so we can pin
    //! `updated_at = updated_at` and bypass MariaDB's `ON UPDATE CURRENT_TIMESTAMP`.
    //!
    //! The typed entity is intentionally provided ahead of its first SeaORM
    //! consumer — admin-crud (#525) and catalog-reader (#524) join against
    //! this table; defining the entity here keeps the schema↔code coupling
    //! in one place and lets downstream PRs add columns to the `Model` shape
    //! without re-deciding the table-name binding.
    #![allow(dead_code)]

    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "metric_catalog")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, column_type = "Binary(16)")]
        pub id: Uuid,
        pub metric_key: String,
        /// One of `ok` / `error` / `unchecked` (DB-side ENUM + CHECK).
        pub schema_status: String,
        pub schema_checked_at: Option<ChronoDateTimeUtc>,
        pub schema_error_code: Option<String>,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod metric_threshold {
    //! `metric_threshold` entity (Refs #519 schema; #525 admin CRUD consumer).
    //!
    //! Mirrors the columns declared in
    //! `migration/m20260522_000002_metric_threshold.rs` minus the two STORED
    //! generated columns (`tenant_id_sentinel`, `is_locked_persisted`) — those
    //! are read-only and reached only via raw SQL in
    //! `domain::admin_threshold::lock_enforcer` and `domain::catalog::resolver`,
    //! both of which already bind UUIDs directly to keep the BINARY(16) shape
    //! explicit.
    //!
    //! `tenant_id` is `Option<Uuid>` because `scope = 'product-default'` rows
    //! carry NULL there (admin CRUD never creates these — only seed-migration
    //! does — but the entity has to model the column shape honestly so
    //! list/get reads succeed against the seeded floor).
    //!
    //! `good` / `warn` / `alert_trigger` / `alert_bad` are DECIMAL(20,6) on the
    //! DB side; we represent them as `f64` here — same trade-off documented at
    //! `domain::catalog::response::ThresholdView` (PRD §12 byte-for-byte gate
    //! is the regression detector if precision ever drifts).
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "metric_threshold")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, column_type = "Binary(16)")]
        pub id: Uuid,
        #[sea_orm(column_type = "Binary(16)", nullable)]
        pub tenant_id: Option<Uuid>,
        pub metric_key: String,
        /// One of `product-default | tenant | role | team | team+role` —
        /// DB-side ENUM, app-side validated through
        /// `domain::admin_threshold::dto::Scope`.
        pub scope: String,
        /// Empty-string sentinel when scope is `product-default` / `tenant`;
        /// non-empty for `role` / `team+role` (CHECK
        /// `chk_metric_threshold_role_slug_shape`).
        pub role_slug: String,
        /// Empty-string sentinel when scope is `product-default` / `tenant` /
        /// `role`; non-empty for `team` / `team+role` (CHECK
        /// `chk_metric_threshold_team_id_shape`).
        pub team_id: String,
        #[sea_orm(column_type = "Decimal(Some((20, 6)))")]
        pub good: f64,
        #[sea_orm(column_type = "Decimal(Some((20, 6)))")]
        pub warn: f64,
        #[sea_orm(column_type = "Decimal(Some((20, 6)))", nullable)]
        pub alert_trigger: Option<f64>,
        #[sea_orm(column_type = "Decimal(Some((20, 6)))", nullable)]
        pub alert_bad: Option<f64>,
        pub is_locked: bool,
        pub locked_by: Option<String>,
        pub locked_at: Option<ChronoDateTimeUtc>,
        pub lock_reason: Option<String>,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod threshold_lock_audit {
    //! `threshold_lock_audit` entity (Refs #519 schema; #525 audit-emitter
    //! is the only writer).
    //!
    //! Append-only: no `Relation`, no `Updatable` flavors; callers use
    //! `ActiveModel` for the single INSERT path. v1 has no read path
    //! either (DESIGN §3.7 line 1089), so the model exists only to give
    //! the audit-emitter typed INSERTs against the same column set the
    //! migration declares.
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "threshold_lock_audit")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, column_type = "Binary(16)")]
        pub id: Uuid,
        /// One of `bypass_attempt | lock_set | lock_cleared` (DB-side ENUM).
        pub event_type: String,
        pub actor_subject: String,
        #[sea_orm(column_type = "Binary(16)")]
        pub tenant_id: Uuid,
        pub metric_key: String,
        pub attempted_scope: Option<String>,
        /// JSON column — captured as a string on the entity to avoid a
        /// sea-orm `Json` column-type dependency for one append-only write
        /// path. The audit-emitter builds the JSON value with
        /// `serde_json::to_string` before insert.
        pub attempted_values: Option<String>,
        pub blocking_scope: Option<String>,
        #[sea_orm(column_type = "Binary(16)", nullable)]
        pub blocking_row_id: Option<Uuid>,
        pub locked_by: Option<String>,
        pub locked_at: Option<ChronoDateTimeUtc>,
        pub lock_reason: Option<String>,
        pub event_at: ChronoDateTimeUtc,
        pub created_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod table_columns {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "table_columns")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub insight_tenant_id: Option<Uuid>,
        pub clickhouse_table: String,
        pub field_name: String,
        pub field_description: Option<String>,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
