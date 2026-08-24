//! `SeaORM` entity definitions for `MariaDB` tables.

pub mod ai_context_entries {
    //! `ai_context_entries` entity — what a tenant or a person wrote down for
    //! the model to read before it explains a metric.
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ai_context_entries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub insight_tenant_id: Uuid,
        pub scope: String,
        pub person_id: Option<Uuid>,
        pub title: String,
        pub body: String,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_credentials {
    //! `ai_credentials` entity — one person's sealed Anthropic token.
    //!
    //! INVARIANT: `ciphertext` opens only through `domain::ai::crypto::open`,
    //! which binds it to this row's tenant and person; no path returns it to a
    //! caller.
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ai_credentials")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub insight_tenant_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub person_id: Uuid,
        pub nonce: Vec<u8>,
        pub ciphertext: Vec<u8>,
        pub hint: String,
        pub created_at: ChronoDateTimeUtc,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ai_settings {
    //! `ai_settings` entity — a tenant's own system prompt, when it has one.
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ai_settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub insight_tenant_id: Uuid,
        pub system_prompt: Option<String>,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

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

pub mod feedback {
    //! `feedback` entity — what a person told us from inside the product.
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "feedback")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub insight_tenant_id: Uuid,
        pub person_id: Uuid,
        pub message: String,
        pub path: String,
        pub created_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
