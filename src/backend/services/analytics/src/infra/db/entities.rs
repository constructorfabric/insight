//! `SeaORM` entity definitions for `MariaDB` tables.

pub mod saved_queries {
    //! `saved_queries` entity — the `presentation.queries` saved query (#1965).
    //!
    //! Metadata in the service DB. `sql` holds a single `SELECT`/`WITH` over
    //! the contract, validated by the query gate on write and run; CRUD never
    //! reaches ClickHouse.
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "saved_queries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub insight_tenant_id: Uuid,
        pub name: String,
        pub description: Option<String>,
        pub sql: String,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
