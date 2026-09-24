use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::AppState;
use crate::domain::definition::{DefinitionKind, DefinitionName, Page};
use crate::domain::kinds::dashboard::Item;
use crate::domain::metric_run::MetricRuns;
use crate::domain::query::time_window::WindowRequest;
use crate::domain::surfaces::{CustomError, Surfaces};
use crate::store::catalog::{CatalogError, TableEntry, TableSchema};

#[cfg(test)]
mod tests;

/// How many tables one description spells out. Columns run long, and an
/// agent that wants more asks again.
const DESCRIBE_LIMIT: usize = 20;

/// How many tables one listing names. A warehouse holds more than a model can
/// read in one answer, so a wider one is cut and said to be cut.
const LIST_LIMIT: usize = 500;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct KindRequest {
    /// Which kind of definition to list.
    pub(crate) kind: DefinitionKind,
    /// How many names to answer with: 1 to 200, 50 by default.
    pub(crate) limit: Option<u64>,
    /// How many names to skip, for the page after the first.
    pub(crate) offset: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchRequest {
    /// Which kind of definition to look through.
    pub(crate) kind: DefinitionKind,
    /// Text to look for in a name or in a stored body — a table name finds
    /// every metric that reads it. Blank returns everything of that kind.
    pub(crate) query: String,
    /// How many names to answer with: 1 to 200, 50 by default.
    pub(crate) limit: Option<u64>,
    /// How many names to skip, for the page after the first.
    pub(crate) offset: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct NamedRequest {
    /// Which kind of definition the name belongs to.
    pub(crate) kind: DefinitionKind,
    /// Letters, digits, underscore and dash, up to 128 characters.
    pub(crate) name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RunRequest {
    /// The stored metric's name.
    pub(crate) name: String,
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
    /// The dashboard to lay out. It must already exist.
    pub(crate) name: String,
    /// Everything the dashboard draws, top to bottom. Each entry names
    /// exactly one of `widget` (a stored widget), `heading` (a section title
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
pub(crate) struct PutRequest {
    /// The name to store under. An existing definition of the same kind and
    /// name is replaced.
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
            tool_router: Self::tool_router(),
        }
    }

    fn surfaces(&self) -> Surfaces<'_> {
        self.state.surfaces()
    }

    fn metric_runs(&self) -> MetricRuns<'_> {
        self.state.metric_runs()
    }

    /// One page of names, with how many there are to page through.
    async fn read_page(
        &self,
        kind: DefinitionKind,
        needle: &str,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> CallToolResult {
        let page = match Page::parse(limit, offset) {
            Ok(page) => page,
            Err(error) => return refuse(&error.to_string()),
        };

        match self.surfaces().page(kind, needle, page).await {
            Ok(found) => CallToolResult::structured(json!({
                "names": found.names,
                "total": found.total,
                "limit": page.limit(),
                "offset": page.offset(),
            })),
            Err(error) => tool_error(&error),
        }
    }

    async fn write(&self, kind: DefinitionKind, request: PutRequest) -> CallToolResult {
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };

        match self.surfaces().put(kind, &name, &request.body).await {
            Ok(()) => CallToolResult::structured(json!({"stored": request.name})),
            Err(error) => tool_error(&error),
        }
    }
}

#[tool_router]
impl CustomSurfaces {
    #[tool(
        name = "list_definitions",
        description = "Names the stored metrics, widgets or dashboards, one page at a time. Start here before writing one, so an existing definition is replaced deliberately rather than by accident. Answers `total`: when it exceeds the page, ask again with `offset`."
    )]
    async fn list_definitions(
        &self,
        Parameters(KindRequest {
            kind,
            limit,
            offset,
        }): Parameters<KindRequest>,
    ) -> CallToolResult {
        self.read_page(kind, "", limit, offset).await
    }

    /// Find definitions by name or by what their body says.
    #[tool(
        description = "Search metrics, widgets or dashboards. Matches the name and the stored body, so a table or column name finds every definition that reads it. Paged like list_definitions, and answers the same `total`."
    )]
    async fn search_definitions(
        &self,
        Parameters(request): Parameters<SearchRequest>,
    ) -> CallToolResult {
        self.read_page(request.kind, &request.query, request.limit, request.offset)
            .await
    }

    #[tool(
        name = "arrange_dashboard",
        description = "Lays out a dashboard that already exists: the order its widgets are drawn in, and the headings and lines of prose between them. Send the whole list top to bottom — it replaces the previous one, and the title is kept. Every widget it names must already exist."
    )]
    async fn arrange_dashboard(
        &self,
        Parameters(request): Parameters<ArrangeRequest>,
    ) -> CallToolResult {
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };

        match self.surfaces().arrange(&name, &request.items).await {
            Ok(body) => CallToolResult::structured(body),
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "get_definition",
        description = "Reads one definition's stored body, so it can be inspected or amended rather than rewritten from scratch."
    )]
    async fn get_definition(
        &self,
        Parameters(NamedRequest { kind, name }): Parameters<NamedRequest>,
    ) -> CallToolResult {
        let parsed = match parse_name(&name) {
            Ok(parsed) => parsed,
            Err(refusal) => return refusal,
        };

        match self.surfaces().get(kind, &parsed).await {
            Ok(body) => CallToolResult::structured(body),
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "put_metric",
        description = "Creates or replaces a metric: a declarative query over one warehouse table or one dataset. Over a table, the body names it as `database.table` - any database, bronze, silver or gold - and reads its columns; a `column` holding JSON is read into with `json`, a dot-separated key path: {\"table\": \"silver.class_ai_assistant_usage\", \"time\": {\"column\": \"day\"}, \"fields\": [{\"column\": \"tool\", \"type\": \"string\", \"as_name\": \"tool\"}, {\"column\": \"surface_metrics_json\", \"json\": \"session_count\", \"type\": \"int\", \"agg\": \"sum\", \"as_name\": \"sessions\"}], \"group_by\": [\"tool\"]}. A replacing table is read through FINAL, so a row is counted once. Over a dataset, the body names the dataset and the fields to read, for example {\"dataset\": \"commits\", \"fields\": [{\"field\": \"author\", \"type\": \"string\", \"as_name\": \"author\"}, {\"field\": \"lines\", \"type\": \"int\", \"agg\": \"sum\", \"as_name\": \"total\"}], \"group_by\": [\"author\"]}. Every `field` names a field the dataset declares; `group_by` and `order_by` name what this metric produces - an `as_name`, or `bucket` for a windowed run. `agg` is count, sum, avg, min or max, and only a number is summed or averaged. `count` alone counts the rows and names no field. Add `time` to window by a field other than the dataset's own main date: {\"time\": {\"field\": \"merged\"}}. `max_range` caps the widest window it will answer, as an ISO duration such as \"P1Y\". Optional `filters`, `order_by` and `limit`. Call list_tables and describe_tables first so the table and its columns exist, or list_datasets so the dataset and its fields do."
    )]
    async fn put_metric(&self, Parameters(request): Parameters<PutRequest>) -> CallToolResult {
        self.write(DefinitionKind::Metric, request).await
    }

    #[tool(
        name = "put_widget",
        description = "Creates or replaces a widget, which draws one metric's columns by the `as_name` that metric gives them: {\"type\": \"table\", \"metric\": \"per-actor\", \"columns\": [\"actor\", \"total\"]} or {\"type\": \"line\", \"metric\": \"per-actor\", \"x\": \"actor\", \"y\": \"total\"}. A widget naming a column its metric does not produce is refused, so run_metric first if unsure what it yields."
    )]
    async fn put_widget(&self, Parameters(request): Parameters<PutRequest>) -> CallToolResult {
        self.write(DefinitionKind::Widget, request).await
    }

    #[tool(
        name = "put_dashboard",
        description = "Creates or replaces a dashboard: a title, and what it draws top to bottom: {\"title\": \"Example board\", \"items\": [{\"heading\": \"Commits\"}, {\"widget\": \"chart\"}, {\"text\": \"Merge commits excluded.\"}]}. Each item names exactly one of `widget`, `heading` or `text`. Add `time_ranges` to let a reader pick the window the whole board is read over - any of `PDC`, `P7D`, `P30D`, `PMC`, `PQC`, `P1Y`, `inf` - and `default_range` for the one it opens on; a board declaring neither is read unbounded. `widgets: [\"chart\"]` is the older shorthand for a list of nothing but widgets, and is still read. To reorder or caption a board that exists, call arrange_dashboard instead."
    )]
    async fn put_dashboard(&self, Parameters(request): Parameters<PutRequest>) -> CallToolResult {
        self.write(DefinitionKind::Dashboard, request).await
    }

    #[tool(
        name = "delete_definition",
        description = "Removes one definition. A metric a widget still draws, or a widget a dashboard still holds, is kept and its dependents named — remove those first."
    )]
    async fn delete_definition(
        &self,
        Parameters(NamedRequest { kind, name }): Parameters<NamedRequest>,
    ) -> CallToolResult {
        let parsed = match parse_name(&name) {
            Ok(parsed) => parsed,
            Err(refusal) => return refusal,
        };

        match self.surfaces().delete(kind, &parsed).await {
            Ok(()) => CallToolResult::structured(json!({"deleted": name})),
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "run_metric",
        description = "Compiles a stored metric and runs it, returning its rows. Use it to answer a question from the data, and to confirm a metric produces the columns a widget will draw. Pass `range` to answer over one window and `bucket: false` to answer it as a single total; a metric with no `time` answers every row and refuses a range."
    )]
    async fn run_metric(
        &self,
        Parameters(RunRequest {
            name,
            range,
            bucket,
        }): Parameters<RunRequest>,
    ) -> CallToolResult {
        let parsed = match parse_name(&name) {
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
        name = "list_datasets",
        description = "Every dataset this server can read, with what each of its fields holds and which records count as one. Call this before writing a metric, so the metric names a dataset and fields that exist. Only an administrator declares a dataset; this server cannot."
    )]
    async fn list_datasets(&self) -> CallToolResult {
        let described = self.state.assistant().briefing().await;

        CallToolResult::structured(json!({ "datasets": described.datasets }))
    }

    #[tool(
        name = "list_tables",
        description = "Every warehouse table a metric may read, as `database` and `table` with the layer it belongs to: bronze is a provider's raw payloads, silver is cleaned per-source models, gold is the published metrics, identity is who people are. Pass `database` to list one database alone; a listing of every database is cut at 500 tables and says so, so narrow it rather than guess at what was left out. Columns are not listed here; call describe_tables for the tables you mean to query. The tables datasets keep their records in are not listed, since a dataset is read by name through list_datasets."
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

        let matching = tables
            .iter()
            .filter(|listed| wanted.is_empty() || listed.database == wanted);
        let total = matching.clone().count();
        let shown: Vec<Value> = matching.take(LIST_LIMIT).map(table_entry).collect();

        CallToolResult::structured(json!({
            "total": total,
            "shown": shown.len(),
            "cut": total > shown.len(),
            "tables": shown,
        }))
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
        let described = match self.state.catalog().describe(&tables, DESCRIBE_LIMIT).await {
            Ok(described) => described,
            Err(error) => return catalog_error(&error),
        };

        let shown: Vec<Value> = described.tables.iter().map(table_description).collect();
        let mut unknown: Vec<&str> = described.unknown.iter().map(String::as_str).collect();
        unknown.sort_unstable();
        unknown.dedup();

        CallToolResult::structured(json!({
            "tables": shown,
            "cut": described.total > shown.len(),
            "unknown": unknown,
        }))
    }
}

fn table_entry(listed: &TableEntry) -> Value {
    json!({
        "database": listed.database,
        "table": listed.table,
        "layer": listed.layer.name(),
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
                 put_widget to draw those rows and put_dashboard to hold the widgets. A metric \
                 names a `table` and its columns, or a dataset and its declared fields; a widget \
                 names its metric's columns by their as_name; and a definition still in use \
                 cannot be deleted until its dependents are. Datasets themselves are declared by \
                 an administrator, not here.",
            )
    }
}

fn parse_name(raw: &str) -> Result<DefinitionName, CallToolResult> {
    DefinitionName::parse(raw).map_err(|error| refuse(&error.to_string()))
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

fn refuse(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_owned())])
}
