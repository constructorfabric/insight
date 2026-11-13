use super::*;

use super::super::super::save_hooks::{self, Gate};

fn changes(output: &str) -> Result<Vec<serde_json::Value>, serde_json::Error> {
    let mut changes = Vec::new();
    for line in output.lines() {
        let mut line: serde_json::Value = serde_json::from_str(line)?;
        if line["fields"]["message"] == "sync job schedule changed" {
            changes.push(line["fields"].take());
        }
    }
    Ok(changes)
}

fn assert_single_change(output: &str) -> R {
    let changes = changes(output)?;
    assert_eq!(changes.len(), 1, "{output}");
    assert_eq!(changes[0]["from_crons"], "[\"0 4 * * *\"]");
    assert_eq!(changes[0]["to_crons"], "[\"0 6 * * *\"]");
    Ok(())
}

async fn resume_confirmation(stub: &Stub, gate: &Gate, scenario: Scenario) {
    let entered =
        tokio::time::timeout(std::time::Duration::from_secs(5), gate.entered.notified()).await;
    assert!(entered.is_ok(), "confirm checkpoint was not reached");

    let object = stub.object().await;
    stub.replace(object, scenario).await;
    gate.resume.notify_one();
}

#[test]
fn a_late_patch_that_wins_the_fence_is_logged_once() -> R {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut result: R = Ok(());
    let output = insight_log_context::test_support::capture_output(|| {
        result = runtime.block_on(async {
            let stub = Stub::start(Scenario::LatePatch(true), "example-late", "main").await?;
            let instance = Instance::parse("example-late", "main")?;
            let (observed, failure) =
                attempt(&stub.cluster, &instance, &["0 6 * * *".to_owned()], 7).await;
            assert!(failure.is_none(), "{failure:?}");
            assert_eq!(observed.live.ok_or("live")?.crons, ["0 6 * * *"]);
            Ok(())
        });
    });
    result?;
    assert_single_change(&output)?;
    Ok(())
}

#[test]
fn an_accepted_patch_confirmed_after_failed_readback_is_logged_once() -> R {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut result: R = Ok(());
    let output = insight_log_context::test_support::capture_output(|| {
        result = runtime.block_on(async {
            let stub = Stub::start(Scenario::Unreadable, "example-readback", "main").await?;
            let instance = Instance::parse("example-readback", "main")?;
            let gate = save_hooks::install("example-readback", "confirm");
            let crons = ["0 6 * * *".to_owned()];
            let ((observed, failure), ()) = tokio::join!(
                attempt(&stub.cluster, &instance, &crons, 7),
                resume_confirmation(&stub, &gate, Scenario::Accept)
            );
            save_hooks::remove("example-readback");
            assert!(failure.is_none(), "{failure:?}");
            assert_eq!(observed.live.ok_or("live")?.crons, ["0 6 * * *"]);
            Ok(())
        });
    });
    result?;
    assert_single_change(&output)?;
    Ok(())
}

#[test]
fn a_failed_fence_without_the_target_logs_no_change() -> R {
    a_late_patch_that_wins_the_fence_is_logged_once()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut result: R = Ok(());
    let output = insight_log_context::test_support::capture_output(|| {
        result = runtime.block_on(async {
            let stub = Stub::start(Scenario::LatePatch(false), "example-fence", "main").await?;
            let instance = Instance::parse("example-fence", "main")?;
            let gate = save_hooks::install("example-fence", "confirm");
            let crons = ["0 6 * * *".to_owned()];
            let ((observed, failure), ()) = tokio::join!(
                attempt(&stub.cluster, &instance, &crons, 7),
                resume_confirmation(&stub, &gate, Scenario::AlwaysConflict)
            );
            save_hooks::remove("example-fence");
            assert!(failure.is_none(), "{failure:?}");
            assert_eq!(observed.live.ok_or("live")?.crons, ["0 4 * * *"]);
            let patches = stub.patches().await;
            assert_eq!(patches.len(), 2);
            assert!(patches[1].get("spec").is_none());
            assert_eq!(patches[1]["metadata"]["resourceVersion"], "7");
            Ok(())
        });
    });
    result?;
    assert!(changes(&output)?.is_empty(), "{output}");
    Ok(())
}
