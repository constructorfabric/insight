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
