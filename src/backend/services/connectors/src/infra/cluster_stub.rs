use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) enum Scenario {
    #[default]
    Accept,
    Refuse(u16),
    Conflict,
    AlwaysConflict,
    Timeout,
    LostResponse,
    LostUnreadable,
    LatePatch(bool),
    Missing,
    AnnotationAbsent,
    Mismatch,
    Unreadable,
    MalformedReadback,
}

#[derive(Debug)]
struct StateData {
    scenario: Scenario,
    object: Value,
    patches: Vec<Value>,
    reconcile: Option<Value>,
    field_managers: Vec<Option<String>>,
    lists: usize,
    pending_patch: Option<Value>,
}

#[derive(Debug)]
pub(crate) struct Stub {
    pub(crate) cluster: super::cluster::Cluster,
    state: Arc<Mutex<StateData>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Stub {
    pub(crate) async fn start(
        scenario: Scenario,
        connector: &str,
        source: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let name = format!(
            "{connector}-{}-tenant01-sync",
            source
                .strip_prefix(&format!("{connector}-"))
                .unwrap_or(source)
        );
        let mut object = json!({"apiVersion":"argoproj.io/v1alpha1", "kind":"CronWorkflow",
            "metadata":{"name":name, "namespace":"example", "uid":"example-uid", "resourceVersion":"7",
                "labels":{"insight.cyberfabric.com/connector":connector, "insight.cyberfabric.com/tenant":"tenant01", "app.kubernetes.io/component":"connector-sync"},
                "annotations":{"insight.cyberfabric.com/schedule-origin":"descriptor", "insight.cyberfabric.com/deployment-schedule":"{\"origin\":\"descriptor\",\"crons\":[\"0 4 * * *\"]}", "example.test/unrelated":"retained"}},
            "spec":{"schedules":["0 4 * * *"], "timezone":"UTC", "suspend":false, "workflowSpec":{"arguments":{"parameters":[{"name":"insight_source_id", "value":source}]}}}});
        if scenario == Scenario::AnnotationAbsent {
            object["metadata"]["annotations"]
                .as_object_mut()
                .ok_or("annotations")?
                .remove("insight.cyberfabric.com/deployment-schedule");
        }
        let state = Arc::new(Mutex::new(StateData {
            scenario,
            object,
            patches: Vec::new(),
            reconcile: None,
            field_managers: Vec::new(),
            lists: 0,
            pending_patch: None,
        }));
        let path = "/apis/argoproj.io/v1alpha1/namespaces/example/cronworkflows";
        let app = Router::new()
            .route(path, get(list))
            .route(&format!("{path}/{{name}}"), get(read).patch(patch))
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let config = kube::Config::new(format!("http://{}", listener.local_addr()?).parse()?);
        let cluster =
            super::cluster::Cluster::new(kube::Client::try_from(config)?, "example", "tenant01");
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            cluster,
            state,
            task,
        })
    }

    pub(crate) async fn lists(&self) -> usize {
        self.state.lock().await.lists
    }

    pub(crate) async fn set_object(&self, object: Value) {
        self.state.lock().await.object = object;
    }

    pub(crate) async fn replace(&self, object: Value, scenario: Scenario) {
        let mut state = self.state.lock().await;
        state.object = object;
        state.scenario = scenario;
    }

    pub(crate) async fn patches(&self) -> Vec<Value> {
        self.state.lock().await.patches.clone()
    }
    pub(crate) async fn field_managers(&self) -> Vec<Option<String>> {
        self.state.lock().await.field_managers.clone()
    }
    pub(crate) async fn set_reconcile(&self, object: Value) {
        self.state.lock().await.reconcile = Some(object);
    }
    pub(crate) async fn object(&self) -> Value {
        self.state.lock().await.object.clone()
    }
    pub(crate) async fn finish_pending_patch(&self) {
        let mut state = self.state.lock().await;
        if let Some(patch) = state.pending_patch.take() {
            commit(&mut state, &patch);
        }
    }
    pub(crate) async fn revert(&self) {
        let mut state = self.state.lock().await;
        state.object["spec"]["schedules"] = json!(["0 4 * * *"]);
        state.object["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"] =
            json!("descriptor");
        state.object["metadata"]["resourceVersion"] = json!("20");
    }
}

fn failure(code: u16) -> Response {
    (StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(json!({
        "apiVersion":"v1", "kind":"Status", "status":"Failure", "message":"synthetic private server detail", "reason": if code == 404 { "NotFound" } else { "Error" }, "code":code
    }))).into_response()
}

async fn read(State(state): State<Arc<Mutex<StateData>>>, Path(name): Path<String>) -> Response {
    let state = state.lock().await;
    if name == "example-reconcile-loop" {
        if let Scenario::Refuse(code) = state.scenario {
            return failure(code);
        }
        return state.reconcile.as_ref().map_or_else(
            || failure(404),
            |object| Json(object.clone()).into_response(),
        );
    }
    if state.scenario == Scenario::Missing {
        return failure(404);
    }
    if matches!(
        state.scenario,
        Scenario::Unreadable | Scenario::LostUnreadable
    ) && !state.patches.is_empty()
    {
        return failure(500);
    }
    let mut object = state.object.clone();
    if state.scenario == Scenario::MalformedReadback && !state.patches.is_empty() {
        object["spec"]["schedules"] = json!(true);
    }
    Json(object).into_response()
}

async fn list(State(state): State<Arc<Mutex<StateData>>>) -> Json<Value> {
    let mut state = state.lock().await;
    let items = if state.scenario == Scenario::Missing {
        Vec::new()
    } else {
        vec![state.object.clone()]
    };
    state.lists += 1;
    Json(
        json!({"apiVersion":"argoproj.io/v1alpha1", "kind":"CronWorkflowList", "metadata":{}, "items":items}),
    )
}

#[derive(Debug, Deserialize)]
struct PatchQuery {
    #[serde(rename = "fieldManager")]
    field_manager: Option<String>,
}

async fn patch(
    State(state): State<Arc<Mutex<StateData>>>,
    Query(params): Query<PatchQuery>,
    Json(patch): Json<Value>,
) -> Response {
    let mut state = state.lock().await;
    state.field_managers.push(params.field_manager);
    state.patches.push(patch.clone());
    let fence = patch.get("spec").is_none();
    if !fence && matches!(state.scenario, Scenario::LatePatch(_)) {
        state.pending_patch = Some(patch);
        return failure(500);
    }
    if fence
        && state.scenario == Scenario::LatePatch(true)
        && let Some(pending) = state.pending_patch.take()
    {
        commit(&mut state, &pending);
    }
    if let Scenario::Refuse(code) = state.scenario
        && !fence
    {
        return failure(code);
    }
    if state.scenario == Scenario::Timeout && !fence {
        drop(state);
        tokio::time::sleep(std::time::Duration::from_secs(11)).await;
        return failure(500);
    }
    if state.scenario == Scenario::AlwaysConflict {
        return failure(409);
    }
    if state.scenario == Scenario::Conflict && state.patches.len() == 1 {
        state.object["metadata"]["resourceVersion"] = json!("8");
        return failure(409);
    }
    if !commit(&mut state, &patch) {
        return failure(409);
    }
    if matches!(
        state.scenario,
        Scenario::LostResponse | Scenario::LostUnreadable
    ) && !fence
    {
        drop(state);
        tokio::time::sleep(std::time::Duration::from_secs(11)).await;
        return failure(500);
    }
    Json(state.object.clone()).into_response()
}

fn commit(state: &mut StateData, patch: &Value) -> bool {
    if patch["metadata"]["uid"] != state.object["metadata"]["uid"]
        || patch["metadata"]["resourceVersion"] != state.object["metadata"]["resourceVersion"]
    {
        return false;
    }
    if patch.get("spec").is_some() && state.scenario != Scenario::Mismatch {
        state.object["spec"]["schedules"] = patch["spec"]["schedules"].clone();
        state.object["spec"]["timezone"] = patch["spec"]["timezone"].clone();
        state.object["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"] =
            patch["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"].clone();
    }
    if let Some(marker) =
        patch["metadata"]["annotations"].get("insight.cyberfabric.com/schedule-fence")
    {
        state.object["metadata"]["annotations"]["insight.cyberfabric.com/schedule-fence"] =
            marker.clone();
    }
    let version = state.object["metadata"]["resourceVersion"]
        .as_str()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or_default()
        + 1;
    state.object["metadata"]["resourceVersion"] = json!(version.to_string());
    true
}
