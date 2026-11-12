use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use tokio::sync::Notify;

use super::Instance;

static GATES: LazyLock<Mutex<HashMap<String, Arc<Gate>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
pub(crate) struct Gate {
    step: &'static str,
    pub(crate) entered: Notify,
    pub(crate) resume: Notify,
    pub(crate) finished: Notify,
}

pub(crate) fn install(connector: &str, step: &'static str) -> Arc<Gate> {
    let gate = Arc::new(Gate {
        step,
        entered: Notify::new(),
        resume: Notify::new(),
        finished: Notify::new(),
    });
    GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(connector.to_owned(), Arc::clone(&gate));
    gate
}

pub(crate) fn remove(connector: &str) {
    GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(connector);
}

pub(crate) async fn checkpoint(instance: &Instance, step: &str) {
    let gate = GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(instance.connector.as_str())
        .cloned();
    let Some(gate) = gate else { return };
    if step == "finished" {
        gate.finished.notify_one();
    }
    if step == gate.step {
        gate.entered.notify_one();
        gate.resume.notified().await;
    }
}
