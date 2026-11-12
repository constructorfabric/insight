pub(crate) mod connector_schedules {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "connector_schedules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub connector: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub source_id: String,
        pub crons: Json,
        pub revision: u64,
        pub updated_by: Uuid,
        pub updated_at: ChronoDateTimeUtc,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
