use sea_orm::Value;

use crate::domain::access::{Caller, Role};

pub(super) type Clause = (String, Vec<Value>);

fn administrator(caller: Caller) -> &'static str {
    match caller.role {
        Role::Administrator => "TRUE",
        Role::Member => "FALSE",
    }
}

fn person(caller: Caller) -> Value {
    caller.person.to_string().into()
}

pub(super) fn folder_visible(folder: &str, caller: Caller) -> Clause {
    let sql = format!(
        "(({folder}.owner_person_id IS NULL AND {admin})
OR {folder}.owner_person_id = ?
OR ({folder}.owner_person_id IS NOT NULL AND EXISTS (SELECT 1 FROM folder_grants {folder}_grants
WHERE {folder}_grants.folder_id = {folder}.id AND {folder}_grants.grantee IN (?, 'everyone'))))",
        admin = administrator(caller),
    );

    (sql, vec![person(caller), person(caller)])
}

fn folder_carrying(folder: &str, caller: Caller) -> Clause {
    let sql = format!(
        "({folder}.owner_person_id IS NOT NULL AND ({folder}.owner_person_id = ?
OR EXISTS (SELECT 1 FROM folder_grants {folder}_grants
WHERE {folder}_grants.folder_id = {folder}.id AND {folder}_grants.grantee IN (?, 'everyone'))))"
    );

    (sql, vec![person(caller), person(caller)])
}

pub(super) fn dashboard_visible(caller: Caller) -> Clause {
    let (carrying, carrying_values) = folder_carrying("visible_folder", caller);
    let sql = format!(
        "((dashboards.owner_person_id IS NULL AND {admin})
OR (dashboards.owner_person_id = ? AND {admin})
OR (dashboards.owner_person_id IS NOT NULL AND EXISTS (SELECT 1 FROM dashboard_grants visible_grants
WHERE visible_grants.dashboard = dashboards.id AND visible_grants.grantee IN (?, 'everyone')))
OR (dashboards.owner_person_id IS NOT NULL AND EXISTS (SELECT 1 FROM folder_entries visible_entries
JOIN folders visible_folder ON visible_folder.id = visible_entries.folder_id
WHERE visible_entries.dashboard = dashboards.id AND visible_entries.carries AND {carrying})))",
        admin = administrator(caller),
    );
    let mut values = vec![person(caller), person(caller)];
    values.extend(carrying_values);

    (sql, values)
}

pub(super) fn dashboard_unfiled(caller: Caller) -> Clause {
    let (visible, values) = folder_visible("unfiled_folder", caller);
    let sql = format!(
        "NOT EXISTS (SELECT 1 FROM folder_entries unfiled_entries
JOIN folders unfiled_folder ON unfiled_folder.id = unfiled_entries.folder_id
WHERE unfiled_entries.dashboard = dashboards.id AND {visible})"
    );

    (sql, values)
}

pub(super) fn dashboard_in(folder: &str, caller: Caller) -> Clause {
    let (visible, values) = folder_visible("filter_folder", caller);
    let sql = format!(
        "EXISTS (SELECT 1 FROM folder_entries filter_entries
JOIN folders filter_folder ON filter_folder.id = filter_entries.folder_id
WHERE filter_entries.dashboard = dashboards.id AND filter_entries.folder_id = {folder}
AND {visible})"
    );

    (sql, values)
}

pub(super) fn administrator_literal(caller: Caller) -> &'static str {
    administrator(caller)
}

#[cfg(test)]
pub(super) fn assert_bound(statement: &sea_orm::Statement) {
    let bound = statement.values.as_ref().map_or(0, |values| values.0.len());

    assert_eq!(
        bound,
        statement.sql.matches('?').count(),
        "{}",
        statement.sql
    );
}

#[cfg(test)]
mod tests;
