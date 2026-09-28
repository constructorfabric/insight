use uuid::Uuid;

use crate::store::definitions::MariaDefinitions;
use crate::store::definitions::migration::{Migrator, name_the_first_migration};

const URL_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";

static MIGRATED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn connect(url: &str) -> sea_orm::DatabaseConnection {
    sea_orm::Database::connect(url)
        .await
        .unwrap_or_else(|error| panic!("{URL_VAR} must reach MariaDB: {error}"))
}

pub(crate) async fn store_or_skip() -> Option<MariaDefinitions> {
    use sea_orm_migration::MigratorTrait as _;

    let url = std::env::var(URL_VAR).unwrap_or_default();
    if url.is_empty() {
        eprintln!("skipping: {URL_VAR} not set");
        return None;
    }

    // WORKAROUND: two migrators racing on a fresh ledger both insert its first
    // row, so the tests in this process migrate once between them.
    MIGRATED
        .get_or_init(|| async {
            let db = connect(&url).await;
            name_the_first_migration(&db)
                .await
                .unwrap_or_else(|error| panic!("the ledger must be readable: {error}"));
            for _ in 0..2 {
                Migrator::up(&db, None).await.unwrap_or_else(|error| {
                    panic!("the migrations must apply twice over: {error}")
                });
            }
        })
        .await;

    Some(MariaDefinitions::new(connect(&url).await))
}

pub(crate) fn unique(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}
