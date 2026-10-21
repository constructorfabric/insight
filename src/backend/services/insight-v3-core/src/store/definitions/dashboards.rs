//! The dashboards one caller sees, as `MariaDB` answers it.

use async_trait::async_trait;
use sea_orm::{FromQueryResult, Statement, Value};

use super::folders::statement;
use super::{MariaDefinitions, TotalRow};
use crate::domain::access::{Owner, PersonId, access};
use crate::domain::dashboards::{
    DashboardCatalogue, DashboardPage, DashboardQuery, ListedDashboard, Owned, Standing,
};
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Page};
use crate::domain::folders::FolderFilter;
use crate::domain::tags::TagFilter;
use crate::store::like_escaped;

/// Whether `dashboards` is a row the caller sees; binds the caller twice.
///
/// INVARIANT: the same rule as [`access`]: the owner, whoever it is shared
/// with, and everyone for an ownerless one. Every query that lists, counts
/// or finds dashboards for a caller carries this clause.
pub(super) const VISIBLE: &str = "(dashboards.owner_person_id IS NULL
OR dashboards.owner_person_id = ?
OR EXISTS (SELECT 1 FROM dashboard_shares
WHERE dashboard_shares.dashboard = dashboards.id AND dashboard_shares.person = ?))";

/// Whether `dashboards` is shared with the caller; binds the caller once.
const SHARED_WITH_CALLER: &str = "EXISTS (SELECT 1 FROM dashboard_shares
WHERE dashboard_shares.dashboard = dashboards.id AND dashboard_shares.person = ?)";

/// Whether `dashboards` is in no folder of the caller's; binds the caller.
pub(super) const UNFILED: &str =
    "((dashboards.owner_person_id IS NULL AND dashboards.folder_id IS NULL)
OR (dashboards.owner_person_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM dashboard_placements
WHERE dashboard_placements.dashboard = dashboards.id AND dashboard_placements.person = ?)))";

/// Whether `dashboards` is in `{folder}` as the caller files it: an
/// ownerless one by its own folder, an owned one by the caller's placement.
/// Binds the caller, and the folder at each `{folder}` that is a `?`.
pub(super) const IN_FOLDER: &str =
    "((dashboards.owner_person_id IS NULL AND dashboards.folder_id = {folder})
OR EXISTS (SELECT 1 FROM dashboard_placements
WHERE dashboard_placements.dashboard = dashboards.id AND dashboard_placements.person = ?
AND dashboard_placements.folder_id = {folder}))";

const TAGGED: &str = "dashboards.id IN (SELECT dashboard_tags.dashboard
FROM dashboard_tags JOIN tags ON tags.id = dashboard_tags.tag_id
WHERE tags.name IN ({names}))";

const STANDING: &str = "SELECT dashboards.owner_person_id, {shared} AS shared
FROM dashboards WHERE dashboards.id = ?";

const PAGE_DASHBOARDS: &str =
    "SELECT dashboards.id, dashboards.name, dashboards.owner_person_id, {shared} AS shared
FROM dashboards
WHERE {where}
ORDER BY dashboards.name, dashboards.id
LIMIT ? OFFSET ?";

const COUNT_DASHBOARDS: &str = "SELECT COUNT(*) AS total FROM dashboards WHERE {where}";

pub(super) fn visible_values(caller: PersonId) -> Vec<Value> {
    vec![caller.to_string().into(), caller.to_string().into()]
}

/// An owner column as the store wrote it.
pub(super) fn owner_of(column: Option<&str>) -> Result<Owner, DefinitionStoreError> {
    match column {
        None => Ok(Owner::Unowned),
        Some(raw) => PersonId::parse(raw)
            .map(Owner::Person)
            .ok_or(DefinitionStoreError::UnreadableRow("owner")),
    }
}

/// An owner as its column holds it: null for ownerless.
pub(super) fn owner_value(owner: Owner) -> Value {
    owner.person().map(|person| person.to_string()).into()
}

#[derive(Debug, FromQueryResult)]
struct StandingRow {
    owner_person_id: Option<String>,
    shared: i64,
}

#[derive(Debug, FromQueryResult)]
struct DashboardRow {
    id: String,
    name: String,
    owner_person_id: Option<String>,
    shared: i64,
}

/// The `WHERE` clause a query keeps, with its values in order.
pub(super) fn narrowed(caller: PersonId, query: &DashboardQuery) -> (String, Vec<Value>) {
    let pattern = format!("%{}%", like_escaped(&query.needle));
    let mut clauses = vec![
        VISIBLE.to_owned(),
        "(dashboards.name LIKE ? OR dashboards.body LIKE ?)".to_owned(),
    ];
    let mut values = visible_values(caller);
    values.push(pattern.clone().into());
    values.push(pattern.into());

    if let Some(owned) = query.owned {
        let (clause, bound) = owned_clause(owned, caller);
        clauses.push(clause);
        values.extend(bound);
    }
    if let Some(filter) = query.folder {
        let (clause, bound) = folder_clause(filter, caller);
        clauses.push(clause);
        values.extend(bound);
    }
    if let Some(tags) = &query.tags {
        let (clause, bound) = tag_clause(tags);
        clauses.push(clause);
        values.extend(bound);
    }

    (clauses.join("\nAND "), values)
}

fn owned_clause(owned: Owned, caller: PersonId) -> (String, Vec<Value>) {
    match owned {
        Owned::Me => (
            "dashboards.owner_person_id = ?".to_owned(),
            vec![caller.to_string().into()],
        ),
        Owned::Shared => (
            format!("dashboards.owner_person_id <> ? AND {SHARED_WITH_CALLER}"),
            vec![caller.to_string().into(), caller.to_string().into()],
        ),
    }
}

pub(super) fn folder_clause(filter: FolderFilter, caller: PersonId) -> (String, Vec<Value>) {
    match filter {
        FolderFilter::Unfiled => (UNFILED.to_owned(), vec![caller.to_string().into()]),
        FolderFilter::In(id) => (
            IN_FOLDER.replace("{folder}", "?"),
            vec![
                id.to_string().into(),
                caller.to_string().into(),
                id.to_string().into(),
            ],
        ),
    }
}

fn tag_clause(tags: &TagFilter) -> (String, Vec<Value>) {
    let names = vec!["?"; tags.names().len()].join(", ");
    let values = tags
        .names()
        .iter()
        .map(|name| name.as_str().into())
        .collect();

    (TAGGED.replace("{names}", &names), values)
}

pub(super) fn page_statement(caller: PersonId, query: &DashboardQuery, page: Page) -> Statement {
    let (clause, narrowing) = narrowed(caller, query);
    let mut values: Vec<Value> = vec![caller.to_string().into()];
    values.extend(narrowing);
    values.push(page.limit().into());
    values.push(page.offset().into());

    let sql = PAGE_DASHBOARDS
        .replace("{shared}", SHARED_WITH_CALLER)
        .replace("{where}", &clause);

    statement(&sql, values)
}

pub(super) fn count_statement(caller: PersonId, query: &DashboardQuery) -> Statement {
    let (clause, values) = narrowed(caller, query);

    statement(&COUNT_DASHBOARDS.replace("{where}", &clause), values)
}

fn listed(
    row: DashboardRow,
    caller: PersonId,
) -> Result<Option<ListedDashboard>, DefinitionStoreError> {
    let owner = owner_of(row.owner_person_id.as_deref())?;
    let Some(access) = access(caller, owner, row.shared != 0) else {
        return Ok(None);
    };

    Ok(Some(ListedDashboard {
        id: row.id,
        name: row.name,
        owner,
        access,
    }))
}

#[async_trait]
impl DashboardCatalogue for MariaDefinitions {
    async fn standing(
        &self,
        dashboard: &DefinitionId,
        caller: PersonId,
    ) -> Result<Option<Standing>, DefinitionStoreError> {
        let sql = STANDING.replace("{shared}", SHARED_WITH_CALLER);
        let row = StandingRow::find_by_statement(statement(
            &sql,
            vec![caller.to_string().into(), dashboard.as_str().into()],
        ))
        .one(&self.db)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(Standing {
            owner: owner_of(row.owner_person_id.as_deref())?,
            shared_with_caller: row.shared != 0,
        }))
    }

    async fn page_dashboards(
        &self,
        caller: PersonId,
        query: &DashboardQuery,
        page: Page,
    ) -> Result<DashboardPage, DefinitionStoreError> {
        let rows = DashboardRow::find_by_statement(page_statement(caller, query, page))
            .all(&self.db)
            .await?;
        let total = TotalRow::find_by_statement(count_statement(caller, query))
            .one(&self.db)
            .await?
            .map_or(0, |row| row.total);

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            items.extend(listed(row, caller)?);
        }

        Ok(DashboardPage {
            items,
            total: u64::try_from(total).unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests;
