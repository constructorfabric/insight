//! Live-DB fixture shared by the identity test suites (the repo-level
//! `visible_set_live_tests` and the API-level `api::http_live_tests`).
//!
//! INVARIANT: tests built on this fixture are never `#[ignore]`d — the identity
//! CI job runs `cargo test` without `--include-ignored`, so an ignored case
//! silently stops running. They skip at runtime via [`fixture_or_skip`].
//! INVARIANT: [`FIXTURE_REASON`] must differ from `e2e-seed` — the e2e seeder
//! deletes by reason with no tenant filter and would wipe fixtures mid-run.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, Value};
use uuid::Uuid;

use super::{connect_single, roles_repo, subchart_repo};

const ENV_VAR: &str = "INTEGRATION_TESTS_MARIADB_URL";
pub(crate) const FIXTURE_REASON: &str = "visible-set-live-test";
pub(crate) const SOURCE_TYPE: &str = "bamboohr";

pub(crate) struct Fixture {
    pub(crate) db: DatabaseConnection,
    pub(crate) tenant: Uuid,
    pub(crate) source_id: Uuid,
}

pub(crate) async fn fixture_or_skip() -> anyhow::Result<Option<Fixture>> {
    let Ok(url) = std::env::var(ENV_VAR) else {
        eprintln!("skip: set {ENV_VAR} to run");
        return Ok(None);
    };
    Ok(Some(Fixture {
        db: connect_single(&url).await?,
        tenant: Uuid::now_v7(),
        source_id: Uuid::now_v7(),
    }))
}

impl Fixture {
    /// A second tenant sharing the connection — for the isolation cases, where
    /// a person must exist somewhere the caller's tenant cannot reach.
    pub(crate) fn in_another_tenant(&self) -> Self {
        Self {
            db: self.db.clone(),
            tenant: Uuid::now_v7(),
            source_id: self.source_id,
        }
    }

    pub(crate) async fn person(&self, email: &str) -> anyhow::Result<Uuid> {
        let person_id = Uuid::now_v7();
        self.exec(
            "INSERT INTO persons (value_type, insight_source_type, insight_source_id,
                 insight_tenant_id, value_id, person_id, author_person_id, reason)
             VALUES ('email', ?, ?, ?, ?, ?, ?, ?)",
            [
                SOURCE_TYPE.into(),
                bytes(self.source_id),
                bytes(self.tenant),
                email.into(),
                bytes(person_id),
                bytes(person_id),
                FIXTURE_REASON.into(),
            ],
        )
        .await?;
        Ok(person_id)
    }

    /// A person the log knows without an email observation — the shape the
    /// `person_id` key exists to serve and the email key structurally cannot.
    pub(crate) async fn emailless_person(&self) -> anyhow::Result<Uuid> {
        let person_id = Uuid::now_v7();
        self.exec(
            "INSERT INTO persons (value_type, insight_source_type, insight_source_id,
                 insight_tenant_id, value_id, person_id, author_person_id, reason)
             VALUES ('id', ?, ?, ?, ?, ?, ?, ?)",
            [
                SOURCE_TYPE.into(),
                bytes(self.source_id),
                bytes(self.tenant),
                format!("acct-{}", person_id.simple()).into(),
                bytes(person_id),
                bytes(person_id),
                FIXTURE_REASON.into(),
            ],
        )
        .await?;
        Ok(person_id)
    }

    pub(crate) async fn reports_to(&self, child: Uuid, parent: Uuid) -> anyhow::Result<()> {
        self.exec(
            "INSERT INTO org_chart (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, parent_person_id, author_person_id, reason, valid_from)
             VALUES (?, ?, ?, ?, ?, ?, ?, UTC_TIMESTAMP(6))",
            [
                bytes(self.tenant),
                SOURCE_TYPE.into(),
                bytes(self.source_id),
                bytes(child),
                bytes(parent),
                bytes(parent),
                FIXTURE_REASON.into(),
            ],
        )
        .await
    }

    /// `target = None` is the wildcard grant: everyone in the tenant.
    pub(crate) async fn grant(&self, viewer: Uuid, target: Option<Uuid>) -> anyhow::Result<()> {
        self.exec(
            "INSERT INTO visibility (visibility_id, insight_tenant_id, viewer_person_id,
                 viewed_person_id, valid_from, author_person_id, reason)
             VALUES (?, ?, ?, ?, UTC_TIMESTAMP(6), ?, ?)",
            [
                bytes(Uuid::now_v7()),
                bytes(self.tenant),
                bytes(viewer),
                target.map_or(Value::Bytes(None), bytes),
                bytes(viewer),
                FIXTURE_REASON.into(),
            ],
        )
        .await
    }

    pub(crate) async fn make_admin(&self, person_id: Uuid) -> anyhow::Result<()> {
        self.exec(
            "INSERT INTO person_roles (person_role_id, insight_tenant_id, person_id, role_id,
                 valid_from, author_person_id, reason)
             VALUES (?, ?, ?, ?, UTC_TIMESTAMP(6), ?, ?)",
            [
                bytes(Uuid::now_v7()),
                bytes(self.tenant),
                bytes(person_id),
                bytes(roles_repo::ADMIN_ROLE_ID),
                bytes(person_id),
                FIXTURE_REASON.into(),
            ],
        )
        .await
    }

    pub(crate) async fn visible(
        &self,
        viewer: Uuid,
        candidates: &[Uuid],
    ) -> anyhow::Result<Vec<Uuid>> {
        subchart_repo::visible_targets(&self.db, self.tenant, viewer, candidates, SOURCE_TYPE).await
    }

    async fn exec(&self, sql: &str, values: impl IntoIterator<Item = Value>) -> anyhow::Result<()> {
        self.db
            .execute(Statement::from_sql_and_values(
                DbBackend::MySql,
                sql,
                values,
            ))
            .await?;
        Ok(())
    }
}

pub(crate) fn bytes(id: Uuid) -> Value {
    id.as_bytes().to_vec().into()
}
