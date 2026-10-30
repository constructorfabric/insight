//! The dashboards one caller sees, as `MariaDB` answers it.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::NaiveDateTime;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement, Value};

use super::folders::statement;
use super::visibility::{
    Clause, administrator_literal, dashboard_in, dashboard_unfiled, dashboard_visible,
};
use super::{MariaDefinitions, TotalRow};
use crate::domain::access::{Caller, DashboardStanding, FolderStanding, Level, Owner, PersonId};
use crate::domain::dashboards::{
    DashboardCatalogue, DashboardPage, DashboardQuery, ListedDashboard, Order, Owned, Seen,
    Visibility,
};
use crate::domain::definition::{DefinitionId, DefinitionStoreError, Page};
use crate::domain::folders::FolderFilter;
use crate::domain::tags::TagFilter;
use crate::store::like_escaped;

const TAGGED: &str = "dashboards.id IN (SELECT dashboard_tags.dashboard
FROM dashboard_tags JOIN tags ON tags.id = dashboard_tags.tag_id
WHERE tags.name IN ({names}))";

const OWNER_OF: &str = "SELECT id, owner_person_id FROM dashboards WHERE id = ?";

// INVARIANT: the carrying terms match `folder_carrying` - a carrying folder shows the dashboard to its owner and its grantees.
const PAGE_DASHBOARDS: &str = "SELECT dashboards.id, dashboards.owner_person_id, dashboards.name,
dashboards.updated_at, JSON_VALUE(dashboards.body, '$.title') AS title,
CASE
WHEN EXISTS (SELECT 1 FROM dashboard_grants WHERE dashboard_grants.dashboard = dashboards.id
AND dashboard_grants.grantee = 'everyone')
OR EXISTS (SELECT 1 FROM folder_entries carrying
JOIN folders carrier ON carrier.id = carrying.folder_id
JOIN folder_grants ON folder_grants.folder_id = carrier.id
WHERE carrying.dashboard = dashboards.id AND carrying.carries
AND carrier.owner_person_id IS NOT NULL AND folder_grants.grantee = 'everyone') THEN 'everyone'
WHEN EXISTS (SELECT 1 FROM dashboard_grants WHERE dashboard_grants.dashboard = dashboards.id)
OR EXISTS (SELECT 1 FROM folder_entries carrying
JOIN folders carrier ON carrier.id = carrying.folder_id
WHERE carrying.dashboard = dashboards.id AND carrying.carries
AND carrier.owner_person_id IS NOT NULL
AND (NOT carrier.owner_person_id <=> dashboards.owner_person_id
OR EXISTS (SELECT 1 FROM folder_grants WHERE folder_grants.folder_id = carrier.id))) THEN 'people'
ELSE 'private'
END AS visibility
FROM dashboards
WHERE {where}
ORDER BY {order}
LIMIT ? OFFSET ?";

const TAGS_OF_PAGE: &str = "SELECT dashboard_tags.dashboard, tags.name
FROM dashboard_tags JOIN tags ON tags.id = dashboard_tags.tag_id
WHERE dashboard_tags.dashboard IN ({ids})
ORDER BY tags.name";

const COUNT_DASHBOARDS: &str = "SELECT COUNT(*) AS total FROM dashboards WHERE {where}";

/// The grants of these dashboards reaching one caller.
const GRANTS_REACHING: &str = "SELECT dashboard, level FROM dashboard_grants
WHERE grantee IN (?, 'everyone') AND dashboard IN ({ids})";

/// Each folder carrying one of these dashboards, with its grants reaching
/// one caller - a row per grant, or one with no level for none.
const CARRIERS: &str = "SELECT carrying.dashboard, carrying.folder_id, carrier.owner_person_id,
carrier_grants.level
FROM folder_entries carrying
JOIN folders carrier ON carrier.id = carrying.folder_id
LEFT JOIN folder_grants carrier_grants ON carrier_grants.folder_id = carrier.id
AND carrier_grants.grantee IN (?, 'everyone')
WHERE carrying.carries AND carrying.dashboard IN ({ids})";

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

/// A level column as the store wrote it.
pub(super) fn level_of(column: &str) -> Result<Level, DefinitionStoreError> {
    Level::parse(column).ok_or(DefinitionStoreError::UnreadableRow("level"))
}

#[derive(Debug, FromQueryResult)]
struct OwnedRow {
    id: String,
    owner_person_id: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct DashboardRow {
    id: String,
    owner_person_id: Option<String>,
    name: String,
    updated_at: NaiveDateTime,
    visibility: String,
    title: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct TagRow {
    dashboard: String,
    name: String,
}

#[derive(Debug, FromQueryResult)]
struct GrantRow {
    dashboard: String,
    level: String,
}

#[derive(Debug, FromQueryResult)]
struct CarrierRow {
    dashboard: String,
    folder_id: String,
    owner_person_id: Option<String>,
    level: Option<String>,
}

/// The `WHERE` clause a query keeps, with its values in order.
pub(super) fn narrowed(caller: Caller, query: &DashboardQuery) -> Clause {
    let pattern = format!("%{}%", like_escaped(&query.needle));
    let (visible, mut values) = dashboard_visible(caller);
    let mut clauses = vec![
        visible,
        "(dashboards.name LIKE ? OR dashboards.body LIKE ?)".to_owned(),
    ];
    values.push(pattern.clone().into());
    values.push(pattern.into());

    let narrowing = [
        query.owned.map(|owned| owned_clause(owned, caller)),
        query.folder.map(|filter| folder_clause(filter, caller)),
        query.tags.as_ref().map(tag_clause),
    ];
    for (clause, bound) in narrowing.into_iter().flatten() {
        clauses.push(clause);
        values.extend(bound);
    }

    (clauses.join("\nAND "), values)
}

fn owned_clause(owned: Owned, caller: Caller) -> Clause {
    let admin = administrator_literal(caller);

    match owned {
        Owned::Me => (
            format!("(dashboards.owner_person_id = ? AND {admin})"),
            vec![caller.person.to_string().into()],
        ),
        Owned::Shared => (
            format!(
                "dashboards.owner_person_id IS NOT NULL AND NOT (dashboards.owner_person_id = ? AND {admin})"
            ),
            vec![caller.person.to_string().into()],
        ),
    }
}

fn folder_clause(filter: FolderFilter, caller: Caller) -> Clause {
    match filter {
        FolderFilter::Unfiled => dashboard_unfiled(caller),
        FolderFilter::In(id) => {
            let (clause, bound) = dashboard_in("?", caller);
            let mut values: Vec<Value> = vec![id.to_string().into()];
            values.extend(bound);

            (clause, values)
        }
    }
}

fn tag_clause(tags: &TagFilter) -> Clause {
    let names = vec!["?"; tags.names().len()].join(", ");
    let values = tags
        .names()
        .iter()
        .map(|name| name.as_str().into())
        .collect();

    (TAGGED.replace("{names}", &names), values)
}

pub(super) fn page_statement(caller: Caller, query: &DashboardQuery, page: Page) -> Statement {
    let (clause, mut values) = narrowed(caller, query);
    values.push(page.limit().into());
    values.push(page.offset().into());

    let order = match query.order {
        Order::Name => "dashboards.name, dashboards.id",
        Order::Recent => "dashboards.updated_at DESC, dashboards.id",
    };
    let sql = PAGE_DASHBOARDS
        .replace("{where}", &clause)
        .replace("{order}", order);

    statement(&sql, values)
}

pub(super) fn count_statement(caller: Caller, query: &DashboardQuery) -> Statement {
    let (clause, values) = narrowed(caller, query);

    statement(&COUNT_DASHBOARDS.replace("{where}", &clause), values)
}

/// A query over `ids` binding the caller first, then each id.
fn over_ids(template: &str, caller: Caller, ids: &[&str]) -> Statement {
    let marks = vec!["?"; ids.len()].join(", ");
    let mut values: Vec<Value> = vec![caller.person.to_string().into()];
    values.extend(ids.iter().map(|id| Value::from(*id)));

    statement(&template.replace("{ids}", &marks), values)
}

async fn tags_of_page<C: ConnectionTrait>(
    db: &C,
    ids: &[&str],
) -> Result<BTreeMap<String, Vec<String>>, DefinitionStoreError> {
    let mut tags: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if ids.is_empty() {
        return Ok(tags);
    }

    let marks = vec!["?"; ids.len()].join(", ");
    let values = ids.iter().map(|id| Value::from(*id)).collect();
    let rows = TagRow::find_by_statement(statement(&TAGS_OF_PAGE.replace("{ids}", &marks), values))
        .all(db)
        .await?;
    for row in rows {
        tags.entry(row.dashboard).or_default().push(row.name);
    }

    Ok(tags)
}

/// What decides `caller`'s access to each of these owned dashboards, read
/// in two queries whatever their number.
pub(super) async fn standings<C: ConnectionTrait>(
    db: &C,
    owners: Vec<(String, Owner)>,
    caller: Caller,
) -> Result<BTreeMap<String, DashboardStanding>, DefinitionStoreError> {
    let mut standings: BTreeMap<String, DashboardStanding> = owners
        .into_iter()
        .map(|(id, owner)| {
            let standing = DashboardStanding {
                owner,
                granted: None,
                carried_by: Vec::new(),
            };
            (id, standing)
        })
        .collect();
    if standings.is_empty() {
        return Ok(standings);
    }
    let ids: Vec<&str> = standings.keys().map(String::as_str).collect();

    let grants = GrantRow::find_by_statement(over_ids(GRANTS_REACHING, caller, &ids))
        .all(db)
        .await?;
    let carriers = CarrierRow::find_by_statement(over_ids(CARRIERS, caller, &ids))
        .all(db)
        .await?;

    for row in grants {
        let level = level_of(&row.level)?;
        if let Some(standing) = standings.get_mut(&row.dashboard) {
            standing.granted = standing.granted.max(Some(level));
        }
    }

    let mut folders: BTreeMap<(String, String), FolderStanding> = BTreeMap::new();
    for row in carriers {
        let owner = owner_of(row.owner_person_id.as_deref())?;
        let level = row.level.as_deref().map(level_of).transpose()?;
        let folder = folders
            .entry((row.dashboard, row.folder_id))
            .or_insert(FolderStanding {
                owner,
                granted: None,
            });
        folder.granted = folder.granted.max(level);
    }
    for ((dashboard, _), folder) in folders {
        if let Some(standing) = standings.get_mut(&dashboard) {
            standing.carried_by.push(folder);
        }
    }

    Ok(standings)
}

#[async_trait]
impl DashboardCatalogue for MariaDefinitions {
    async fn standing(
        &self,
        dashboard: &DefinitionId,
        caller: Caller,
    ) -> Result<Option<DashboardStanding>, DefinitionStoreError> {
        let row = OwnedRow::find_by_statement(statement(OWNER_OF, vec![dashboard.as_str().into()]))
            .one(&self.db)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };

        let owner = owner_of(row.owner_person_id.as_deref())?;
        let mut held = standings(&self.db, vec![(row.id, owner)], caller).await?;

        Ok(held.pop_first().map(|(_, standing)| standing))
    }

    async fn page_dashboards(
        &self,
        caller: Caller,
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

        let mut owners = Vec::with_capacity(rows.len());
        for row in &rows {
            owners.push((row.id.clone(), owner_of(row.owner_person_id.as_deref())?));
        }
        let standings = standings(&self.db, owners, caller).await?;
        let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        let mut tags = tags_of_page(&self.db, &ids).await?;

        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(seen) = standings
                .get(&row.id)
                .and_then(|standing| Seen::of(caller, standing))
            else {
                continue;
            };
            let visibility = Visibility::parse(&row.visibility)
                .ok_or(DefinitionStoreError::UnreadableRow("visibility"))?;

            let tags = tags.remove(&row.id).unwrap_or_default();

            items.push(ListedDashboard {
                id: row.id,
                name: row.name,
                owner: seen.owner,
                access: seen.access,
                updated_at: row.updated_at.and_utc(),
                visibility,
                title: row.title,
                tags,
            });
        }

        Ok(DashboardPage {
            items,
            total: u64::try_from(total).unwrap_or(0),
        })
    }
}

#[cfg(test)]
mod tests;
