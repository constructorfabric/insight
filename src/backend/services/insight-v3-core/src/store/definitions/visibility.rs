//! The clauses keeping only the dashboards and folders one caller sees.
//!
//! INVARIANT: each keeps exactly the rows [`crate::domain::access::access`]
//! and [`crate::domain::access::folder_access`] answer some access for. Every
//! query that lists, counts or finds dashboards or folders for a caller
//! carries one of them.

use sea_orm::Value;

use crate::domain::access::{Caller, Role};

/// A clause with the values its `?`s bind, in order.
pub(super) type Clause = (String, Vec<Value>);

/// The role as a SQL literal: substituted from our own enum, never from
/// anything a request carries.
fn administrator(caller: Caller) -> &'static str {
    match caller.role {
        Role::Administrator => "TRUE",
        Role::Member => "FALSE",
    }
}

fn person(caller: Caller) -> Value {
    caller.person.to_string().into()
}

/// Whether the folder row aliased `folder` is one `caller` sees: an
/// ownerless one for an administrator, their own, or one granted to them or
/// to everyone. Aliases its grants `{folder}_grants`.
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

/// Whether the owned folder row aliased `folder` gives `caller` something
/// to carry: their own, or one granted to them or to everyone.
fn folder_carrying(folder: &str, caller: Caller) -> Clause {
    let sql = format!(
        "({folder}.owner_person_id IS NOT NULL AND ({folder}.owner_person_id = ?
OR EXISTS (SELECT 1 FROM folder_grants {folder}_grants
WHERE {folder}_grants.folder_id = {folder}.id AND {folder}_grants.grantee IN (?, 'everyone'))))"
    );

    (sql, vec![person(caller), person(caller)])
}

/// Whether the `dashboards` row is one `caller` sees: an ownerless one for
/// an administrator, their own while they are one, one granted to them or to
/// everyone, or one a folder they see carries.
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

/// Whether the `dashboards` row is in none of the folders `caller` sees.
pub(super) fn dashboard_unfiled(caller: Caller) -> Clause {
    let (visible, values) = folder_visible("unfiled_folder", caller);
    let sql = format!(
        "NOT EXISTS (SELECT 1 FROM folder_entries unfiled_entries
JOIN folders unfiled_folder ON unfiled_folder.id = unfiled_entries.folder_id
WHERE unfiled_entries.dashboard = dashboards.id AND {visible})"
    );

    (sql, values)
}

/// Whether the `dashboards` row is in the folder `{folder}`, a `?` or a
/// column, which `caller` sees. Binds the folder first when it is a `?`.
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

/// Whether the role literal of `caller` keeps an administrator's rows.
pub(super) fn administrator_literal(caller: Caller) -> &'static str {
    administrator(caller)
}

/// Whether a statement binds a value to every `?` it holds, and no more.
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
