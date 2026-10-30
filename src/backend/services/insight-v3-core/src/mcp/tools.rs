use std::sync::Arc;

use axum::http::request::Parts;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::auth::McpCaller;
use crate::api::AppState;
use crate::api::definitions::{dashboards_json, listed_json, owner_json};
use crate::api::folders::{folder_json, folders_json, holding_json};
use crate::api::tags::{names_json, tags_json};
use crate::domain::access::{Caller, FolderAccess, PersonId};
use crate::domain::dashboards::DashboardQuery;
use crate::domain::dataset_lifecycle::DatasetChangeError;
use crate::domain::definition::{DefinitionId, DefinitionKind, DefinitionLabel, Page};
use crate::domain::folders::{self, FolderError, FolderId, FolderName, SeenFolder};
use crate::domain::kinds::dashboard::Item;
use crate::domain::metric_run::MetricRuns;
use crate::domain::query::time_window::WindowRequest;
use crate::domain::surfaces::{CustomError, Surfaces};
use crate::domain::tags::{self, TagError, TagSet};
use crate::domain::violation::said;
use crate::store::catalog::{CatalogError, TableSchema};

#[cfg(test)]
mod tests;

/// How many tables one description spells out. Columns run long, and an
/// agent that wants more asks again.
const DESCRIBE_LIMIT: usize = 20;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct KindRequest {
    /// Which kind of definition to list.
    pub(crate) kind: DefinitionKind,
    /// How many definitions to answer with: 1 to 200, 50 by default.
    pub(crate) limit: Option<u64>,
    /// How many definitions to skip, for the page after the first.
    pub(crate) offset: Option<u64>,
    /// Dashboards only: a folder id from `list_folders`, or `unfiled` for the
    /// dashboards in no folder. Leave it out to list every one.
    pub(crate) folder: Option<String>,
    /// Dashboards only: tag names from `list_tags`, to list the dashboards
    /// carrying any of them. Combines with `folder`.
    pub(crate) tags: Option<Vec<String>>,
    /// Dashboards only: `me` for your own dashboards, `shared` for those
    /// that reach you through a grant or a folder. Leave it out to list every
    /// one you see.
    pub(crate) owned: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchRequest {
    /// Which kind of definition to look through.
    pub(crate) kind: DefinitionKind,
    /// Text to look for in a name or in a stored body — a table name finds
    /// every metric that reads it. Blank returns everything of that kind.
    pub(crate) query: String,
    /// How many definitions to answer with: 1 to 200, 50 by default.
    pub(crate) limit: Option<u64>,
    /// How many definitions to skip, for the page after the first.
    pub(crate) offset: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct IdRequest {
    /// Which kind of definition the id belongs to.
    pub(crate) kind: DefinitionKind,
    /// The definition's id, as `list_definitions` and `search_definitions`
    /// answer it - not its name.
    pub(crate) id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RunRequest {
    /// The stored metric's id, as `list_definitions` answers it.
    pub(crate) id: String,
    /// Which window to answer over: `PDC` (the last complete day), `P7D`,
    /// `P30D`, `PMC` (the last complete calendar month), `PQC` (the last
    /// complete calendar quarter), `P1Y`, `inf` (every dated row), or an
    /// ISO 8601 date interval such as `2026-08-01/2026-09-01`, whose end
    /// date is excluded. Relative windows count back from now, so one that
    /// starts after the newest row answers no rows rather than sliding back
    /// to the last day that has them. Omit it to read every row, as a run
    /// with no options always has.
    pub(crate) range: Option<String>,
    /// Whether the answer comes one row per time bucket. `false` answers one
    /// row for the whole window, which is what a total is. `true` by
    /// default.
    pub(crate) bucket: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ArrangeRequest {
    /// The id of the dashboard to lay out. It must already exist.
    pub(crate) id: String,
    /// Everything the dashboard draws, top to bottom. Each entry names
    /// exactly one of `widget` (a stored widget's id), `heading` (a section title
    /// over the widgets that follow) or `text` (a line of prose between
    /// them).
    pub(crate) items: Vec<Item>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct TablesRequest {
    /// Only this database's tables. Leave it out to list every database.
    pub(crate) database: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct DescribeTablesRequest {
    /// Tables as `database.table`, at most 20 at a time. A bare table name
    /// means it in every database that has one.
    pub(crate) tables: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct FolderRequest {
    /// 1 to 64 characters.
    pub(crate) name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct EntryRequest {
    /// The id of the dashboard.
    pub(crate) dashboard: String,
    /// The folder's id from `list_folders`.
    pub(crate) folder: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct TagsRequest {
    /// The id of the dashboard to tag. It must already exist.
    pub(crate) id: String,
    /// Every tag the dashboard carries from now on, replacing what it had:
    /// each 1 to 32 characters, at most 10. `[]` removes them all.
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PutRequest {
    /// The dataset's name. An existing dataset of that name is replaced.
    pub(crate) name: String,
    /// The declaration.
    pub(crate) body: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WriteRequest {
    /// The id of the definition to replace, as `list_definitions` answers it.
    /// Leave it out to create a new definition, whose id is answered.
    pub(crate) id: Option<String>,
    /// What a reader calls it: 1 to 128 characters, free text. Names are not
    /// unique, so reusing one creates a second definition rather than
    /// replacing the first.
    pub(crate) name: String,
    /// The definition body.
    pub(crate) body: Value,
}

#[derive(Clone)]
pub(crate) struct CustomSurfaces {
    state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

impl std::fmt::Debug for CustomSurfaces {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CustomSurfaces").finish()
    }
}

impl CustomSurfaces {
    pub(crate) fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router() + Self::alert_tool_router(),
        }
    }

    pub(super) fn state(&self) -> &AppState {
        &self.state
    }

    fn surfaces(&self, caller: Caller) -> Surfaces<'_> {
        self.state.surfaces(caller)
    }

    fn metric_runs(&self) -> MetricRuns<'_> {
        self.state.metric_runs()
    }

    /// One page of ids and names, with how many there are to page through.
    async fn read_page(
        &self,
        caller: Caller,
        kind: DefinitionKind,
        asked: Asked<'_>,
    ) -> CallToolResult {
        let page = match Page::parse(asked.limit, asked.offset) {
            Ok(page) => page,
            Err(error) => return refuse(&error.to_string()),
        };
        let query = match DashboardQuery::parse(
            kind,
            asked.needle,
            asked.folder,
            asked.tags,
            asked.owned,
            None,
        ) {
            Ok(query) => query,
            Err(error) => return refuse(&error.to_string()),
        };

        let surfaces = self.surfaces(caller);
        let found = match kind.library() {
            Some(library) => surfaces
                .page(library, &query.needle, page)
                .await
                .map(|found| (listed_json(&found.items), found.total)),
            None => surfaces
                .page_dashboards(&query, page)
                .await
                .map(|found| (dashboards_json(&found.items, None), found.total)),
        };

        match found {
            Ok((items, total)) => CallToolResult::structured(json!({
                "items": items,
                "total": total,
                "limit": page.limit(),
                "offset": page.offset(),
            })),
            Err(error) => tool_error(&error),
        }
    }

    async fn write(
        &self,
        parts: &Parts,
        kind: DefinitionKind,
        request: WriteRequest,
    ) -> CallToolResult {
        let caller = match subject(parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let name = match DefinitionLabel::parse(&request.name) {
            Ok(name) => name,
            Err(error) => return refuse(&error.to_string()),
        };
        let surfaces = self.surfaces(caller);
        let Some(raw) = request.id else {
            return match surfaces.create(kind, &name, &request.body).await {
                Ok(id) => written(&id, &name, true),
                Err(error) => tool_error(&error),
            };
        };

        let id = match parse_id(kind, &raw) {
            Ok(id) => id,
            Err(refusal) => return refusal,
        };
        match surfaces.replace(kind, &id, &name, &request.body).await {
            Ok(()) => written(&id, &name, false),
            Err(error) => tool_error(&error),
        }
    }

    /// A dashboard as `get_definition` answers it, with how the caller sees
    /// it, the folders they see holding it and its tags.
    async fn read_dashboard(&self, caller: Caller, id: &DefinitionId) -> CallToolResult {
        let surfaces = self.surfaces(caller);
        let stored = match surfaces.read(DefinitionKind::Dashboard, id).await {
            Ok(stored) => stored,
            Err(error) => return tool_error(&error),
        };
        let seen = match surfaces.seen(id).await {
            Ok(seen) => seen,
            Err(error) => return tool_error(&error),
        };
        let folders = match self.state.folders().folders_holding(id, caller).await {
            Ok(folders) => folders,
            Err(error) => return folder_refusal(&error),
        };

        match self.state.tags().tags_of(id).await {
            Ok(tags) => CallToolResult::structured(json!({
                "id": id.as_str(),
                "name": stored.name,
                "body": stored.body,
                "owner_person_id": owner_json(seen.owner),
                "access": seen.access.as_str(),
                "carries": seen.entry.carries(),
                "folders": folders.iter().map(holding_json).collect::<Vec<_>>(),
                "tags": names_json(&tags),
            })),
            Err(error) => tag_refusal(&error),
        }
    }
}

/// What one listing asked for.
#[derive(Debug, Clone, Copy)]
struct Asked<'a> {
    needle: &'a str,
    limit: Option<u64>,
    offset: Option<u64>,
    folder: Option<&'a str>,
    tags: &'a [String],
    owned: Option<&'a str>,
}

#[tool_router]
impl CustomSurfaces {
    #[tool(
        name = "list_definitions",
        description = "Lists the stored metrics, widgets or dashboards as `items`, each with its `id` and its `name`, one page at a time, ordered by name. Every other tool addresses a definition by its id; the name is a label people read and two definitions may share one. Start here before writing, so an existing definition is replaced by its id rather than duplicated under the same name. Answers `total`: when it exceeds the page, ask again with `offset`. Pass `folder` to list only the dashboards in one folder, or `unfiled` for those in none, and `tags` to list only the dashboards carrying any of them; the two combine."
    )]
    async fn list_definitions(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<KindRequest>,
    ) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let tags = request.tags.unwrap_or_default();
        let asked = Asked {
            needle: "",
            limit: request.limit,
            offset: request.offset,
            folder: request.folder.as_deref(),
            tags: &tags,
            owned: request.owned.as_deref(),
        };

        self.read_page(caller, request.kind, asked).await
    }

    /// Find definitions by name or by what their body says.
    #[tool(
        description = "Search metrics, widgets or dashboards. Matches the name and the stored body, so a table or column name finds every definition that reads it. Answers `items` with each match's `id` and `name`, paged like list_definitions, and the same `total`."
    )]
    async fn search_definitions(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<SearchRequest>,
    ) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let asked = Asked {
            needle: &request.query,
            limit: request.limit,
            offset: request.offset,
            folder: None,
            tags: &[],
            owned: None,
        };

        self.read_page(caller, request.kind, asked).await
    }

    #[tool(
        name = "arrange_dashboard",
        description = "Lays out a dashboard that already exists, named by its `id`: the order its widgets are drawn in, and the headings and lines of prose between them. Send the whole list top to bottom — it replaces the previous one, and the title and name are kept. Every widget it names, by id, must already exist."
    )]
    async fn arrange_dashboard(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<ArrangeRequest>,
    ) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let id = match parse_id(DefinitionKind::Dashboard, &request.id) {
            Ok(id) => id,
            Err(refusal) => return refusal,
        };

        match self.surfaces(caller).arrange(&id, &request.items).await {
            Ok(body) => CallToolResult::structured(body),
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "get_definition",
        description = "Reads one definition by its `id`, answering its `id`, its `name` and its stored `body`, so it can be inspected or amended rather than rewritten from scratch. A dashboard also answers the `folders` you see holding it and the `tags` it carries. Neither is part of the body: add_to_folder and remove_from_folder change the folders and set_dashboard_tags the tags."
    )]
    async fn get_definition(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(IdRequest { kind, id }): Parameters<IdRequest>,
    ) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let parsed = match parse_id(kind, &id) {
            Ok(parsed) => parsed,
            Err(refusal) => return refusal,
        };
        if kind == DefinitionKind::Dashboard {
            return self.read_dashboard(caller, &parsed).await;
        }

        match self.surfaces(caller).read(kind, &parsed).await {
            Ok(stored) => CallToolResult::structured(json!({
                "id": id,
                "name": stored.name,
                "body": stored.body,
            })),
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "put_metric",
        description = "Creates a metric, or replaces one: pass `name`, the label people read, and `body`; without `id` a new metric is created and its `id` answered, with the `id` of a stored metric that metric is replaced. A metric is a declarative query over one warehouse table or one dataset. Over a table, the body names it as `database.table` - any database, bronze, silver or gold - and reads its columns; a `column` holding JSON is read into with `json`, a dot-separated key path: {\"table\": \"silver.class_ai_assistant_usage\", \"time\": {\"column\": \"day\"}, \"fields\": [{\"column\": \"tool\", \"type\": \"string\", \"as_name\": \"tool\"}, {\"column\": \"surface_metrics_json\", \"json\": \"session_count\", \"type\": \"int\", \"agg\": \"sum\", \"as_name\": \"sessions\"}], \"group_by\": [\"tool\"]}. A replacing table is read through FINAL, so a row is counted once. Over a dataset, the body names the dataset and the fields to read, for example {\"dataset\": \"commits\", \"fields\": [{\"field\": \"author\", \"type\": \"string\", \"as_name\": \"author\"}, {\"field\": \"lines\", \"type\": \"int\", \"agg\": \"sum\", \"as_name\": \"total\"}], \"group_by\": [\"author\"]}. Every `field` names a field the dataset declares; `group_by` and `order_by` name what this metric produces - an `as_name`, or `bucket` for a windowed run. `agg` is count, sum, avg, min or max, and only a number is summed or averaged. `count` alone counts the rows and names no field. Add `time` to window by a field other than the dataset's own main date: {\"time\": {\"field\": \"merged\"}}. `max_range` caps the widest window it will answer, as an ISO duration such as \"P1Y\". Optional `filters`, `order_by` and `limit`. Call list_tables and describe_tables first so the table and its columns exist, or list_datasets so the dataset and its fields do."
    )]
    async fn put_metric(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<WriteRequest>,
    ) -> CallToolResult {
        self.write(&parts, DefinitionKind::Metric, request).await
    }

    #[tool(
        name = "put_widget",
        description = "Creates a widget, or replaces one: pass `name` and `body`, and the `id` of a stored widget to replace it; without `id` a new widget is created and its `id` answered. A widget draws one metric, named by the metric's id, by the `as_name` that metric gives its columns: {\"type\": \"table\", \"metric\": \"<metric id>\", \"columns\": [\"actor\", \"total\"]} or {\"type\": \"line\", \"metric\": \"<metric id>\", \"x\": \"actor\", \"y\": \"total\"}. A widget naming a column its metric does not produce is refused, so run_metric first if unsure what it yields."
    )]
    async fn put_widget(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<WriteRequest>,
    ) -> CallToolResult {
        self.write(&parts, DefinitionKind::Widget, request).await
    }

    #[tool(
        name = "put_dashboard",
        description = "Creates a dashboard, or replaces one: pass `name` and `body`, and the `id` of a stored dashboard to replace it; without `id` a new dashboard is created and its `id` answered. The body is a title, and what it draws top to bottom: {\"title\": \"Example board\", \"items\": [{\"heading\": \"Commits\"}, {\"widget\": \"<widget id>\"}, {\"text\": \"Merge commits excluded.\"}]}. Each item names exactly one of `widget` (a stored widget's id), `heading` or `text`. Add `time_ranges` to let a reader pick the window the whole board is read over - any of `PDC`, `P7D`, `P30D`, `PMC`, `PQC`, `P1Y`, `inf` - and `default_range` for the one it opens on; a board declaring neither is read unbounded. `widgets: [\"<widget id>\"]` is the older shorthand for a list of nothing but widgets, and is still read. To reorder or caption a board that exists, call arrange_dashboard instead."
    )]
    async fn put_dashboard(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<WriteRequest>,
    ) -> CallToolResult {
        self.write(&parts, DefinitionKind::Dashboard, request).await
    }

    #[tool(
        name = "list_folders",
        description = "The folders you see - your own, those shared with you and the ownerless ones - each with its id, its owner (null when ownerless), your `access` and how many of the dashboards you see it holds, and how many you see are in none of them. Pass a folder's id to list_definitions to list its dashboards, or to add_to_folder to put one there."
    )]
    async fn list_folders(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };

        match self.state.folders().list_folders(caller).await {
            Ok(listed) => CallToolResult::structured(folders_json(&listed)),
            Err(error) => folder_refusal(&error),
        }
    }

    #[tool(
        name = "create_folder",
        description = "Makes a dashboard folder of your own and answers its id. The name is 1 to 64 characters; two folders may share one, so call list_folders first to reuse one. Folders are renamed and removed in the portal, not here."
    )]
    async fn create_folder(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(FolderRequest { name }): Parameters<FolderRequest>,
    ) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let name = match FolderName::parse(&name) {
            Ok(name) => name,
            Err(error) => return folder_refusal(&error),
        };

        match self
            .state
            .folders()
            .create_folder(caller.person, name)
            .await
        {
            Ok(folder) => CallToolResult::structured(folder_json(&SeenFolder {
                folder,
                access: FolderAccess::Owner,
            })),
            Err(error) => folder_refusal(&error),
        }
    }

    #[tool(
        name = "add_to_folder",
        description = "Puts a dashboard, named by its id, in a folder, named by its id from list_folders, that you own or edit. A dashboard may sit in any number of folders. Where you own the dashboard or hold an editor grant on it, the entry carries: whoever sees the folder sees the dashboard. Otherwise it only files it there for those who see it anyway. A dashboard with an owner goes in a folder with an owner, an ownerless one in an ownerless folder. Answers whether the entry carries. The dashboard's body is untouched."
    )]
    async fn add_to_folder(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(EntryRequest { dashboard, folder }): Parameters<EntryRequest>,
    ) -> CallToolResult {
        let (caller, parsed, into) = match entry_of(&parts, &dashboard, &folder) {
            Ok(entry) => entry,
            Err(refusal) => return refusal,
        };

        match folders::add(self.state.store(), caller, into, &parsed).await {
            Ok(entry) => CallToolResult::structured(json!({
                "dashboard": dashboard,
                "folder": folder,
                "carries": entry.carries(),
            })),
            Err(error) => folder_refusal(&error),
        }
    }

    #[tool(
        name = "remove_from_folder",
        description = "Takes a dashboard, named by its id, out of a folder, named by its id from list_folders, that you own or edit. The dashboard itself, and its place in every other folder, stay."
    )]
    async fn remove_from_folder(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(EntryRequest { dashboard, folder }): Parameters<EntryRequest>,
    ) -> CallToolResult {
        let (caller, parsed, from) = match entry_of(&parts, &dashboard, &folder) {
            Ok(entry) => entry,
            Err(refusal) => return refusal,
        };

        match folders::remove(self.state.store(), caller, from, &parsed).await {
            Ok(()) => CallToolResult::structured(json!({
                "dashboard": dashboard,
                "folder": folder,
            })),
            Err(error) => folder_refusal(&error),
        }
    }

    #[tool(
        name = "list_tags",
        description = "Every tag name on the dashboards you see, with how many of those carry it. Pass tag names to list_definitions to list the dashboards carrying any of them, or to set_dashboard_tags to tag one."
    )]
    async fn list_tags(&self, Extension(parts): Extension<Parts>) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };

        match self.state.tags().list_tags(caller).await {
            Ok(listed) => CallToolResult::structured(tags_json(&listed)),
            Err(error) => tag_refusal(&error),
        }
    }

    #[tool(
        name = "set_dashboard_tags",
        description = "Replaces every tag a dashboard, named by its `id`, carries with `tags`: at most 10, each 1 to 32 characters. Tags belong to the dashboard's owner: a name equal to one of the owner's tags but for case is that tag, under its stored spelling, so call list_tags first; a new name makes a tag, and a tag no dashboard carries any more goes away. At most 200 tags exist per owner. `[]` removes them all. Only its owner or an editor tags a dashboard. The dashboard's body is untouched. Answers the tags the dashboard now carries."
    )]
    async fn set_dashboard_tags(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(TagsRequest { id, tags }): Parameters<TagsRequest>,
    ) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let parsed = match parse_id(DefinitionKind::Dashboard, &id) {
            Ok(parsed) => parsed,
            Err(refusal) => return refusal,
        };
        let wanted = match TagSet::parse(&tags) {
            Ok(wanted) => wanted,
            Err(error) => return tag_refusal(&error),
        };

        if let Err(error) = tags::retag(self.state.store(), caller, &parsed, &wanted).await {
            return tag_refusal(&error);
        }

        match self.state.tags().tags_of(&parsed).await {
            Ok(carried) => CallToolResult::structured(json!({
                "dashboard": id,
                "tags": names_json(&carried),
            })),
            Err(error) => tag_refusal(&error),
        }
    }

    #[tool(
        name = "delete_definition",
        description = "Removes one definition by its `id`. A metric a widget still draws, or a widget a dashboard still holds, is kept and its dependents named — remove those first."
    )]
    async fn delete_definition(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(IdRequest { kind, id }): Parameters<IdRequest>,
    ) -> CallToolResult {
        let caller = match subject(&parts) {
            Ok(caller) => caller,
            Err(refusal) => return refusal,
        };
        let parsed = match parse_id(kind, &id) {
            Ok(parsed) => parsed,
            Err(refusal) => return refusal,
        };

        match self.surfaces(caller).delete(kind, &parsed).await {
            Ok(()) => CallToolResult::structured(json!({"deleted": id})),
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "run_metric",
        description = "Compiles a stored metric, named by its `id`, and runs it, returning its rows. Use it to answer a question from the data, and to confirm a metric produces the columns a widget will draw. Pass `range` to answer over one window and `bucket: false` to answer it as a single total; a metric with no `time` answers every row and refuses a range."
    )]
    async fn run_metric(
        &self,
        Parameters(RunRequest { id, range, bucket }): Parameters<RunRequest>,
    ) -> CallToolResult {
        let parsed = match parse_id(DefinitionKind::Metric, &id) {
            Ok(parsed) => parsed,
            Err(refusal) => return refusal,
        };

        let requested = match WindowRequest::parse(range.as_deref(), bucket) {
            Ok(requested) => requested,
            Err(error) => return refuse(&error.to_string()),
        };

        match self.metric_runs().run(&parsed, &requested).await {
            Ok(result) => match serde_json::to_value(&result) {
                Ok(value) => CallToolResult::structured(value),
                Err(error) => {
                    tracing::error!(%error, "a metric result could not be encoded");
                    refuse("the metric ran but its result could not be encoded")
                }
            },
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "put_dataset",
        description = "Declares a dataset, or replaces the declaration of one that stands: {\"title\": \"Commits\", \"fields\": [{\"name\": \"day\", \"path\": \"day\", \"type\": \"datetime\", \"default_clock\": true}, {\"name\": \"lines\", \"path\": \"lines\", \"type\": \"int\"}], \"row_identity\": [\"day\"]}. `type` is string, int, float, bool or datetime; `path` is where the value sits in a record, dot-separated. `default_clock` marks the date a window selects by. `row_identity` names the fields that make two records the same one; leave it out and every record stands alone. Optional per field: `role` (dimension, measurable, time), `description`, `absent_value`, `person` (email or id). A replacement that would change what a stored metric reads is refused and names the metric."
    )]
    async fn put_dataset(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<PutRequest>,
    ) -> CallToolResult {
        let Some(McpCaller(caller)) = parts.extensions.get::<McpCaller>() else {
            return refuse("only an administrator declares a dataset");
        };

        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };

        tracing::info!(%caller, dataset = name.as_str(), "declaring a dataset over MCP");

        match self
            .state
            .dataset_lifecycle()
            .declare(&name, &request.body)
            .await
        {
            Ok(stored) => CallToolResult::structured(json!({
                "name": request.name,
                "declaration": stored,
            })),
            Err(error) => change_refusal(error),
        }
    }

    #[tool(
        name = "list_datasets",
        description = "Every dataset this server can read, with what each of its fields holds and which records count as one. Call this before writing a metric, so the metric names a dataset and fields that exist."
    )]
    async fn list_datasets(&self) -> CallToolResult {
        let described = self.state.assistant().described_datasets().await;

        CallToolResult::structured(json!({ "datasets": described }))
    }

    #[tool(
        name = "list_tables",
        description = "Every warehouse table a metric may read, as `database` and `table` with the layer it belongs to: bronze is a provider's raw payloads, silver is cleaned per-source models, gold is the published metrics, identity is who people are. Columns are not listed here; call describe_tables for the tables you mean to query. The tables datasets keep their records in are not listed, since a dataset is read by name through list_datasets."
    )]
    async fn list_tables(
        &self,
        Parameters(TablesRequest { database }): Parameters<TablesRequest>,
    ) -> CallToolResult {
        let wanted = database.unwrap_or_default();
        let wanted = wanted.trim();
        let tables = match self.state.catalog().tables().await {
            Ok(tables) => tables,
            Err(error) => return catalog_error(&error),
        };

        let listed: Vec<Value> = tables
            .iter()
            .filter(|schema| wanted.is_empty() || schema.database == wanted)
            .map(table_entry)
            .collect();

        CallToolResult::structured(json!({ "total": listed.len(), "tables": listed }))
    }

    #[tool(
        name = "describe_tables",
        description = "The columns of the named warehouse tables, each with its type, and the engine holding the table; a replacing engine is read through FINAL without the metric saying so. Name a table as `database.table`, at most 20 at a time. A name the warehouse does not hold is reported under `unknown` rather than left out."
    )]
    async fn describe_tables(
        &self,
        Parameters(DescribeTablesRequest { tables }): Parameters<DescribeTablesRequest>,
    ) -> CallToolResult {
        if tables.is_empty() {
            return refuse("name at least one table as `database.table`");
        }
        if tables.len() > DESCRIBE_LIMIT {
            return refuse(&format!(
                "describe at most {DESCRIBE_LIMIT} tables at a time; {} were named",
                tables.len()
            ));
        }
        let described = match self.state.catalog().describe(&tables).await {
            Ok(described) => described,
            Err(error) => return catalog_error(&error),
        };

        let unknown: Vec<&String> = tables
            .iter()
            .filter(|name| !described.iter().any(|schema| schema.is_named(name)))
            .collect();
        let shown: Vec<Value> = described.iter().map(table_description).collect();

        CallToolResult::structured(json!({ "tables": shown, "unknown": unknown }))
    }
}

fn table_entry(schema: &TableSchema) -> Value {
    json!({
        "database": schema.database,
        "table": schema.table,
        "layer": schema.layer.name(),
    })
}

fn table_description(schema: &TableSchema) -> Value {
    let columns: Vec<Value> = schema
        .columns
        .iter()
        .map(|column| json!({ "name": column.name, "type": column.kind }))
        .collect();

    json!({
        "database": schema.database,
        "table": schema.table,
        "layer": schema.layer.name(),
        "engine": schema.engine,
        "columns": columns,
    })
}

/// A catalogue that did not answer. The wait is the caller's to retry; what
/// the warehouse said is logged, not answered.
fn catalog_error(error: &CatalogError) -> CallToolResult {
    match error {
        CatalogError::Timeout => refuse(&error.to_string()),
        CatalogError::ClickHouse(source) => {
            tracing::error!(error = %source, "the warehouse catalogue could not be read");
            refuse("the warehouse catalogue could not be read")
        }
    }
}

#[tool_router(router = alert_tool_router)]
impl CustomSurfaces {
    #[tool(
        name = "list_alerts",
        description = "Lists the stored alerts by id and name, one page at a time, or the ones whose name or metric matches `query`. Answers `total`: when it exceeds the page, ask again with `offset`."
    )]
    async fn list_alerts(
        &self,
        Parameters(request): Parameters<super::alerts::ListAlertsRequest>,
    ) -> CallToolResult {
        self.alerts_list(request).await
    }

    #[tool(
        name = "preview_alert",
        description = "Checks a metric, window and condition as an alert would right now, without saving anything: answers `breach`, `no_breach` or `unknown` with the reason, and the rows that matched. Takes the same `condition` create_alert does; a refusal names the path of what is wrong."
    )]
    async fn preview_alert(
        &self,
        Parameters(request): Parameters<super::alerts::PreviewAlertRequest>,
    ) -> CallToolResult {
        self.alerts_preview(request).await
    }

    #[tool(
        name = "get_alert",
        description = "Reads one alert by id: its rule, its revision, and what its latest check found. The revision is what update_alert and set_alert_enabled expect."
    )]
    async fn get_alert(
        &self,
        Parameters(request): Parameters<super::alerts::AlertIdRequest>,
    ) -> CallToolResult {
        self.alerts_get(request).await
    }

    #[tool(
        name = "create_alert",
        description = "Creates an alert and answers its id. A rule has a name people read, the id of a stored metric, a condition, how often to check in seconds, and a configured destination from list_alert_destinations; `range` runs the metric over a window as run_metric does. The condition names which rows must pass (`rows`: `one`, `any`, `all`, `none`, or `{\"count\": {\"op\": \">\", \"value\": 3}}`) and the test each row takes (`where`): a test `{\"column\", \"op\", \"value\"}` on a result column by its `as_name`, or a group `{\"all\": [...]}`, `{\"any\": [...]}`, `{\"not\": ...}`. Example: `{\"rows\": \"any\", \"where\": {\"all\": [{\"column\": \"open\", \"op\": \">\", \"value\": 10}, {\"column\": \"team\", \"op\": \"in\", \"value\": [\"core\", \"infra\"]}]}}`. Operators depend on what the column holds, and a refusal names the path of what is wrong. A test on a cell it cannot read makes the check unknown, never a breach. A notification is owed on the first check that meets the condition and again only after a check has seen it clear."
    )]
    async fn create_alert(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<super::alerts::CreateAlertRequest>,
    ) -> CallToolResult {
        self.alerts_create(&parts, request).await
    }

    #[tool(
        name = "update_alert",
        description = "Replaces an alert's whole rule, its condition included, at the revision get_alert answered; a rule that has moved on is refused. Replacing resets what its checks found, so the next breach is notified again. Leave `enabled` out to keep it as it is."
    )]
    async fn update_alert(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<super::alerts::UpdateAlertRequest>,
    ) -> CallToolResult {
        self.alerts_update(&parts, request).await
    }

    #[tool(
        name = "set_alert_enabled",
        description = "Turns an alert's checks on or off at the revision it is at. Turning it off withdraws the notifications it has not sent; either resets what its checks found. Turning on is refused while the alert's destination is no longer configured."
    )]
    async fn set_alert_enabled(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<super::alerts::SetAlertEnabledRequest>,
    ) -> CallToolResult {
        self.alerts_set_enabled(&parts, request).await
    }

    #[tool(
        name = "delete_alert",
        description = "Removes an alert by id, with its schedule and every notification recorded for it."
    )]
    async fn delete_alert(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(request): Parameters<super::alerts::AlertIdRequest>,
    ) -> CallToolResult {
        self.alerts_delete(&parts, request).await
    }

    #[tool(
        name = "list_alert_notifications",
        description = "The notifications an alert's checks have owed, newest first, each with the condition it was owed under, the rows the check saw that met it, and whether it is still pending."
    )]
    async fn list_alert_notifications(
        &self,
        Parameters(request): Parameters<super::alerts::AlertNotificationsRequest>,
    ) -> CallToolResult {
        self.alerts_notifications(request).await
    }

    #[tool(
        name = "list_alert_destinations",
        description = "The destinations an alert may send to, by name, with the provider behind each. Call this before create_alert."
    )]
    async fn list_alert_destinations(&self) -> CallToolResult {
        self.alerts_destinations()
    }
}

#[expect(
    clippy::unused_async_trait_impl,
    reason = "the ServerHandler body is rmcp's, not ours"
)]
#[tool_handler(router = self.tool_router)]
impl ServerHandler for CustomSurfaces {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("insight-custom-surfaces", env!("CARGO_PKG_VERSION"))
                    .with_title("Insight custom surfaces"),
            )
            .with_instructions(
                "Author the metrics, widgets and dashboards the portal reads. Call \
                 list_datasets and list_tables to learn what data exists, describe_tables for \
                 the columns of the tables you mean to query, put_metric to define a query over \
                 one warehouse table or one dataset, run_metric to see the rows it yields, then \
                 put_widget to draw those rows and put_dashboard to hold the widgets. Metrics, \
                 widgets and dashboards are addressed by the id a put answers; their name is a \
                 label, and two may share one. A metric names a `table` and its columns, or a \
                 dataset and its declared fields; a widget names its metric by id and the \
                 metric's columns by their as_name; a dashboard names its widgets by id; and a \
                 definition still in use \
                 cannot be deleted until its dependents are. put_dataset declares a dataset for \
                 metrics to read. Dashboards sit in folders: list_folders shows \
                 them, create_folder makes one, and add_to_folder and remove_from_folder put a \
                 dashboard in one or take it out. Dashboards also carry tags: list_tags shows them, and \
                 set_dashboard_tags replaces a dashboard's set. create_alert watches the rows a metric \
                 produces and owes a notification when they meet a condition; \
                 list_alert_destinations names where one may go.",
            )
    }
}

/// Who is asking, as the token's subject names them. Every call reaches a
/// tool authenticated, so a subject that is not a person is refused, not
/// guessed at.
///
/// INVARIANT: the MCP surface admits administrators alone, so its caller is
/// one.
fn subject(parts: &Parts) -> Result<Caller, CallToolResult> {
    parts
        .extensions
        .get::<McpCaller>()
        .and_then(|McpCaller(subject)| PersonId::parse(subject))
        .map(Caller::administrator)
        .ok_or_else(|| refuse("only an administrator reaches the definitions"))
}

/// The caller, dashboard and folder an entry tool names.
fn entry_of(
    parts: &Parts,
    dashboard: &str,
    folder: &str,
) -> Result<(Caller, DefinitionId, FolderId), CallToolResult> {
    let caller = subject(parts)?;
    let parsed = parse_id(DefinitionKind::Dashboard, dashboard)?;
    let folder = FolderId::parse(folder).map_err(|error| folder_refusal(&error))?;

    Ok((caller, parsed, folder))
}

fn parse_name(raw: &str) -> Result<DefinitionId, CallToolResult> {
    DefinitionId::parse(raw).map_err(|error| refuse(&error.to_string()))
}

/// An id a tool was given. One that cannot be an id names nothing.
fn parse_id(kind: DefinitionKind, raw: &str) -> Result<DefinitionId, CallToolResult> {
    DefinitionId::parse(raw)
        .map_err(|_| refuse(&format!("{} `{raw}` was not found", kind.singular())))
}

fn written(id: &DefinitionId, name: &DefinitionLabel, created: bool) -> CallToolResult {
    CallToolResult::structured(json!({
        "id": id.as_str(),
        "name": name.as_str(),
        "created": created,
    }))
}

fn tool_error(error: &CustomError) -> CallToolResult {
    // A refusal the caller caused is theirs to read; a failure of ours says
    // what the warehouse or the store said, which is not theirs to see.
    if error.is_about_the_caller() {
        return refuse(&error.to_string());
    }

    tracing::error!(%error, "an MCP tool call failed");

    refuse("the request could not be completed")
}

fn folder_refusal(error: &FolderError) -> CallToolResult {
    if error.is_about_the_caller() {
        return refuse(&error.to_string());
    }

    tracing::error!(%error, "an MCP folder call failed");

    refuse("the request could not be completed")
}

fn tag_refusal(error: &TagError) -> CallToolResult {
    if error.is_about_the_caller() {
        return refuse(&error.to_string());
    }

    tracing::error!(%error, "an MCP tag call failed");

    refuse("the request could not be completed")
}

fn change_refusal(error: DatasetChangeError) -> CallToolResult {
    match error {
        DatasetChangeError::Invalid(violations) => refuse(&format!(
            "the declaration is not valid: {}",
            said(&violations)
        )),
        DatasetChangeError::WouldBreak(broken) => refuse(&format!(
            "the replacement would change what these metrics read: {}",
            broken
                .iter()
                .map(|one| format!("{}: {}", one.metric, one.why))
                .collect::<Vec<_>>()
                .join("; ")
        )),
        DatasetChangeError::Table(_)
        | DatasetChangeError::Store(_)
        | DatasetChangeError::Definitions(_) => {
            tracing::error!(%error, "an MCP dataset declaration failed");
            refuse("the request could not be completed")
        }
        about_the_caller @ (DatasetChangeError::NotFound
        | DatasetChangeError::StillRead(_)
        | DatasetChangeError::Unreadable(_)
        | DatasetChangeError::Refused(_)) => refuse(&about_the_caller.to_string()),
    }
}

pub(super) fn refuse(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_owned())])
}
