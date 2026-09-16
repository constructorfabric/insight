use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::AppState;
use crate::definitions::{DefinitionKind, DefinitionName, Page};
use crate::domain::kinds::dashboard::Item;
use crate::domain::query::time_window::WindowRequest;
use crate::domain::surfaces::{CustomError, Surfaces};
use crate::store::catalog::{Layer, TableSchema};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ToolKind {
    Metric,
    Widget,
    Dashboard,
}

impl ToolKind {
    fn kind(self) -> DefinitionKind {
        match self {
            Self::Metric => DefinitionKind::Metric,
            Self::Widget => DefinitionKind::Widget,
            Self::Dashboard => DefinitionKind::Dashboard,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct KindRequest {
    /// Which kind of definition to list.
    pub(crate) kind: ToolKind,
    /// How many names to answer with: 1 to 200, 50 by default.
    pub(crate) limit: Option<u64>,
    /// How many names to skip, for the page after the first.
    pub(crate) offset: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SearchRequest {
    /// Which kind of definition to look through.
    pub(crate) kind: ToolKind,
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
    pub(crate) kind: ToolKind,
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
pub(crate) struct PutRequest {
    /// The name to store under. An existing definition of the same kind and
    /// name is replaced.
    pub(crate) name: String,
    /// The definition body.
    pub(crate) body: Value,
}

#[derive(Debug, Serialize)]
struct TableEntry {
    database: String,
    table: String,
    layer: &'static str,
    columns: Vec<ColumnEntry>,
}

#[derive(Debug, Serialize)]
struct ColumnEntry {
    name: String,
    r#type: String,
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

    /// One page of names, with how many there are to page through.
    async fn read_page(
        &self,
        kind: ToolKind,
        needle: &str,
        limit: Option<u64>,
        offset: Option<u64>,
    ) -> CallToolResult {
        let page = match Page::parse(limit, offset) {
            Ok(page) => page,
            Err(error) => return refuse(&error.to_string()),
        };

        match self.surfaces().page(kind.kind(), needle, page).await {
            Ok(found) => CallToolResult::structured(json!({
                "names": found.names,
                "total": found.total,
                "limit": page.limit(),
                "offset": page.offset(),
            })),
            Err(error) => tool_error(&error),
        }
    }

    async fn write(&self, kind: ToolKind, request: PutRequest) -> CallToolResult {
        let name = match parse_name(&request.name) {
            Ok(name) => name,
            Err(refusal) => return refusal,
        };

        match self.surfaces().put(kind.kind(), &name, &request.body).await {
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

        match self.surfaces().get(kind.kind(), &parsed).await {
            Ok(body) => CallToolResult::structured(body),
            Err(error) => tool_error(&error),
        }
    }

    #[tool(
        name = "put_metric",
        description = "Creates or replaces a metric: a declarative query over an ingested table. The body names the table and the fields to read, for example {\"table\": \"events\", \"fields\": [{\"json\": \"actor\", \"type\": \"string\", \"as_name\": \"actor\"}, {\"json\": \"actor\", \"type\": \"string\", \"agg\": \"count\", \"as_name\": \"total\"}], \"group_by\": [\"actor\"]}. A field reads a typed column of the table (`column`), a key inside the row's `raw_data` payload (`json`), or a key inside any JSON column the table carries (`column` and `json` together, as in {\"column\": \"field_values_json\", \"json\": \"name\"}). A `json` key may be a dotted path into nested objects, such as \"field.name\". When the payload is an array of objects, add `where` - one filter, shaped like the others - to say which element the field means: {\"column\": \"field_values_json\", \"json\": \"name\", \"type\": \"string\", \"as_name\": \"status\", \"where\": {\"json\": \"field.name\", \"type\": \"string\", \"op\": \"eq\", \"value\": \"Status\"}}. `type` is string, int or float; `agg` is count, sum, avg, min or max. Add `time` to say which timestamp a reader may window by - {\"time\": {\"column\": \"occurred_at\"}} for a date column, or {\"json\": \"committed_at\"} for one inside the payload - and the run gains a `bucket` column ordered oldest first. `max_range` caps the widest window it will answer, as an ISO duration such as \"P1Y\". Optional `database`, `filters`, `order_by` and `limit`. Call list_tables first so the table and columns exist."
    )]
    async fn put_metric(&self, Parameters(request): Parameters<PutRequest>) -> CallToolResult {
        self.write(ToolKind::Metric, request).await
    }

    #[tool(
        name = "put_widget",
        description = "Creates or replaces a widget, which draws one metric's columns by the `as_name` that metric gives them: {\"type\": \"table\", \"metric\": \"per-actor\", \"columns\": [\"actor\", \"total\"]} or {\"type\": \"line\", \"metric\": \"per-actor\", \"x\": \"actor\", \"y\": \"total\"}. A widget naming a column its metric does not produce is refused, so run_metric first if unsure what it yields."
    )]
    async fn put_widget(&self, Parameters(request): Parameters<PutRequest>) -> CallToolResult {
        self.write(ToolKind::Widget, request).await
    }

    #[tool(
        name = "put_dashboard",
        description = "Creates or replaces a dashboard: a title, and what it draws top to bottom: {\"title\": \"Example board\", \"items\": [{\"heading\": \"Commits\"}, {\"widget\": \"chart\"}, {\"text\": \"Merge commits excluded.\"}]}. Each item names exactly one of `widget`, `heading` or `text`. Add `time_ranges` to let a reader pick the window the whole board is read over - any of `PDC`, `P7D`, `P30D`, `PMC`, `PQC`, `P1Y`, `inf` - and `default_range` for the one it opens on; a board declaring neither is read unbounded. `widgets: [\"chart\"]` is the older shorthand for a list of nothing but widgets, and is still read. To reorder or caption a board that exists, call arrange_dashboard instead."
    )]
    async fn put_dashboard(&self, Parameters(request): Parameters<PutRequest>) -> CallToolResult {
        self.write(ToolKind::Dashboard, request).await
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

        match self.surfaces().delete(kind.kind(), &parsed).await {
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

        match self.surfaces().run_metric(&parsed, &requested).await {
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
        name = "list_tables",
        description = "Every database and table this server can see, each with its columns and the layer it belongs to. Call this before writing a metric, so the metric names a table and columns that exist."
    )]
    async fn list_tables(&self) -> CallToolResult {
        let tables = match self.surfaces().tables().await {
            Ok(tables) => tables,
            Err(error) => return tool_error(&error),
        };

        let entries: Vec<TableEntry> = tables.iter().map(table_entry).collect();

        match serde_json::to_value(&entries) {
            Ok(value) => CallToolResult::structured(json!({"tables": value})),
            Err(error) => {
                tracing::error!(%error, "the table catalogue could not be encoded");
                refuse("the catalogue was read but could not be encoded")
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CustomSurfaces {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("insight-custom-surfaces", env!("CARGO_PKG_VERSION"))
                    .with_title("Insight custom surfaces"),
            )
            .with_instructions(
                "Author the metrics, widgets and dashboards the portal reads. Call list_tables \
                 to learn what data exists, put_metric to define a query over it, run_metric to \
                 see the rows it yields, then put_widget to draw those rows and put_dashboard to \
                 hold the widgets. A widget names its metric's columns by their as_name, and a \
                 definition still in use cannot be deleted until its dependents are.",
            )
    }
}

fn table_entry(schema: &TableSchema) -> TableEntry {
    TableEntry {
        database: schema.database.clone(),
        table: schema.table.clone(),
        layer: layer_name(schema.layer),
        columns: schema
            .columns
            .iter()
            .map(|(name, kind)| ColumnEntry {
                name: name.clone(),
                r#type: kind.clone(),
            })
            .collect(),
    }
}

fn layer_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Bronze => "bronze",
        Layer::Silver => "silver",
        Layer::Gold => "gold",
        Layer::Identity => "identity",
        Layer::Ingest => "ingest",
        Layer::Other => "other",
    }
}

fn parse_name(raw: &str) -> Result<DefinitionName, CallToolResult> {
    DefinitionName::parse(raw).map_err(|error| refuse(&error.to_string()))
}

fn tool_error(error: &CustomError) -> CallToolResult {
    if !error.is_about_the_caller() {
        tracing::error!(%error, "an MCP tool call failed");
    }

    refuse(&error.to_string())
}

fn refuse(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.to_owned())])
}
