use std::sync::Arc;

use tokio::sync::{Barrier, Notify};

use super::*;
use crate::domain::connector_schedules::{AFTER_WRITE_READ, AFTER_WRITE_READ_RELEASE};

#[tokio::test]
#[ignore = "requires INTEGRATION_TESTS_MARIADB_URL"]
async fn a_noop_put_never_overwrites_a_real_put() -> R {
    let Some(db) = connect_or_skip().await? else {
        return Ok(());
    };
    for real_first in [true, false] {
        let (connector, uri) = identity();
        let sync_history = Mock::new();
        let h = harness(
            db.clone(),
            sync_history.url(),
            true,
            context("user", &[])?,
            true,
        )
        .await?;
        let target = (&h, &sync_history);
        assert_eq!(
            send(target, "PUT", &uri, Some("\"0\""), &["0 6 * * *"])
                .await?
                .0,
            StatusCode::OK
        );
        let before = Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .ok_or("original row missing")?;
        let barrier = Arc::new(Barrier::new(3));
        let real_release = Arc::new(Notify::new());
        let noop_release = Arc::new(Notify::new());
        let first_done = Notify::new();
        let real = async {
            let result = AFTER_WRITE_READ
                .scope(
                    barrier.clone(),
                    AFTER_WRITE_READ_RELEASE.scope(
                        real_release.clone(),
                        send(target, "PUT", &uri, Some("\"1\""), &["0 9 * * *"]),
                    ),
                )
                .await;
            if real_first {
                first_done.notify_one();
            }
            result
        };
        let noop = async {
            let result = AFTER_WRITE_READ
                .scope(
                    barrier.clone(),
                    AFTER_WRITE_READ_RELEASE.scope(
                        noop_release.clone(),
                        send(target, "PUT", &uri, Some("\"1\""), &["0 6 * * *"]),
                    ),
                )
                .await;
            if !real_first {
                let unchanged = Entity::find_by_id((connector.clone(), "main".to_owned()))
                    .one(&db)
                    .await?;
                assert_eq!(unchanged.as_ref(), Some(&before));
                first_done.notify_one();
            }
            result
        };
        let order = async {
            barrier.wait().await;
            let (first, second) = if real_first {
                (&real_release, &noop_release)
            } else {
                (&noop_release, &real_release)
            };
            first.notify_one();
            first_done.notified().await;
            second.notify_one();
        };
        let (real, noop, ()) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::join!(real, noop, order)
        })
        .await?;
        let (real_status, real) = real?;
        let (noop_status, noop) = noop?;
        let row = Entity::find_by_id((connector.clone(), "main".to_owned()))
            .one(&db)
            .await?
            .ok_or("winner missing")?;
        clear(&db, &connector).await?;
        assert_eq!(noop_status, StatusCode::OK, "real_first={real_first}");
        assert_eq!(noop["revision"], 1, "real_first={real_first}");
        assert_eq!(noop["requested"]["crons"], json!(["0 6 * * *"]));
        assert_eq!(
            noop["requested"]["updated_at"],
            serde_json::to_value(before.updated_at)?
        );
        assert_eq!(real_status, StatusCode::OK);
        assert_eq!(real["revision"], 2);
        assert_eq!(row.revision, 2);
        assert_eq!(row.crons, json!(["0 9 * * *"]));
    }
    Ok(())
}
