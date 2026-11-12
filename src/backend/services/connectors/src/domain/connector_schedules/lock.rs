use sea_orm::DatabaseConnection;
use sea_orm::sqlx::{MySql, pool::PoolConnection};

use super::{Error, Instance};

#[derive(Debug)]
pub(crate) struct InstanceLock {
    connection: PoolConnection<MySql>,
    name: String,
}

pub(crate) async fn acquire(
    db: &DatabaseConnection,
    instance: &Instance,
) -> Result<InstanceLock, Error> {
    let mut connection = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        db.get_mysql_connection_pool().acquire(),
    )
    .await
    .map_err(|_| Error::LockBusy)??;
    connection.close_on_drop();
    let key = format!(
        "{}/{}",
        instance.connector.as_str(),
        instance.source_id.as_str()
    );
    let name: String = bounded(
        1,
        sea_orm::sqlx::query_scalar("SELECT SHA2(CONCAT(DATABASE(), '/', ?), 256)")
            .bind(key)
            .fetch_one(&mut *connection),
    )
    .await?;
    let acquired: Option<i64> = bounded(
        6,
        sea_orm::sqlx::query_scalar(
            "SET STATEMENT max_statement_time=6 FOR SELECT CAST(GET_LOCK(?, 5) AS SIGNED)",
        )
        .bind(&name)
        .fetch_one(&mut *connection),
    )
    .await?;
    if acquired != Some(1) {
        return Err(Error::LockBusy);
    }
    Ok(InstanceLock { connection, name })
}

impl InstanceLock {
    pub(crate) async fn release(mut self) -> Result<(), Error> {
        let released: Option<i64> = bounded(
            1,
            sea_orm::sqlx::query_scalar("SELECT CAST(RELEASE_LOCK(?) AS SIGNED)")
                .bind(&self.name)
                .fetch_one(&mut *self.connection),
        )
        .await?;
        if released != Some(1) {
            return Err(Error::LockBusy);
        }
        Ok(())
    }
}

async fn bounded<T>(
    seconds: u64,
    operation: impl std::future::Future<Output = Result<T, sea_orm::sqlx::Error>>,
) -> Result<T, Error> {
    tokio::time::timeout(std::time::Duration::from_secs(seconds), operation)
        .await
        .map_err(|_| sea_orm::sqlx::Error::PoolTimedOut)?
        .map_err(Error::from)
}
