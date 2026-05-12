use anyhow::Result;
use clap::Parser;

mod core;
#[cfg(feature = "io")]
mod io;

#[derive(Parser, Debug)]
#[command(name = "jira-enrich", about = "Jira Silver enrich — materializes task_tracker_field_history.")]
struct Args {
    /// Connector instance scope (e.g. jira-alpha).
    #[arg(long, env = "INSIGHT_SOURCE_ID")]
    insight_source_id: String,

    /// ClickHouse host.
    #[arg(long, env = "CLICKHOUSE_HOST")]
    clickhouse_host: String,

    /// ClickHouse HTTP port (the `clickhouse` crate drives the HTTP interface).
    /// Default 8123 matches the CH HTTP default; pass a different value only when
    /// operators expose CH HTTP on a non-default port (e.g. 8443 behind a proxy).
    #[arg(long, env = "CLICKHOUSE_PORT", default_value_t = 8123)]
    clickhouse_port: u16,

    #[arg(long, env = "CLICKHOUSE_USER", default_value = "default")]
    clickhouse_user: String,

    /// Rows per INSERT batch. Larger = fewer HTTP round-trips, more memory per batch.
    #[arg(long, default_value_t = 50_000)]
    batch_size: usize,

    /// Number of concurrent writer tasks. Each owns its own HTTP connection to ClickHouse.
    /// 3–4 is a good balance on a local Kind cluster; bump to 8–16 for production CH.
    #[arg(long, default_value_t = 4)]
    writers: usize,

    /// Bounded channel capacity between reader task and main loop.
    #[arg(long, default_value_t = 50_000)]
    events_channel_capacity: usize,

    /// Bounded channel capacity between main loop and each writer (per writer).
    #[arg(long, default_value_t = 4)]
    writer_queue_depth: usize,

    /// Per-batch INSERT timeout in seconds. A CH-side error on INSERT (e.g. schema mismatch)
    /// can leave the `clickhouse` crate's chunked-POST future hanging — this guard surfaces
    /// the failure within a predictable window instead of burning the whole
    /// `activeDeadlineSeconds` of the pod.
    #[arg(long, default_value_t = 60)]
    insert_timeout_secs: u64,

    /// Do not write; log row counts.
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    init_tracing();

    tracing::info!(
        insight_source_id = %args.insight_source_id,
        clickhouse_host = %args.clickhouse_host,
        batch_size = args.batch_size,
        writers = args.writers,
        events_channel_capacity = args.events_channel_capacity,
        writer_queue_depth = args.writer_queue_depth,
        insert_timeout_secs = args.insert_timeout_secs,
        dry_run = args.dry_run,
        "jira-enrich starting"
    );

    #[cfg(feature = "io")]
    {
        run(args).await?;
    }

    #[cfg(not(feature = "io"))]
    {
        tracing::warn!(
            "binary built without `io` feature — core module is available but no ClickHouse \
             work will be performed. Build with `--features io` for the full binary."
        );
        let _ = args;
    }

    tracing::info!("jira-enrich finished");
    Ok(())
}

fn init_tracing() {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().json().with_env_filter(env_filter).init();
}

#[cfg(feature = "io")]
async fn run(args: Args) -> Result<()> {
    use crate::core::process_issue;
    use crate::core::types::{
        DeltaEvent, FieldHistoryRecord, FieldMeta, IssueSnapshot, LastState,
    };
    use crate::io::ch_client::ChConfig;
    use crate::io::{reader, schema, writer};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    let password = std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_default();
    let cfg = ChConfig {
        host: args.clickhouse_host,
        port: args.clickhouse_port,
        user: args.clickhouse_user,
        password,
        // Rust `jira-enrich` reads and writes only `staging.jira__*` tables. Silver layer
        // (`silver.class_task_*`) is materialized by dbt via `union_by_tag`.
        database: "staging".into(),
    };

    schema::validate_field_history(&cfg).await?;

    // --- Phase 1: prefetch reference data. ---
    let meta = reader::fetch_field_metadata(&cfg, &args.insight_source_id).await?;
    tracing::info!(fields = meta.len(), "loaded field metadata");

    let hwms = reader::per_issue_hwm(&cfg, &args.insight_source_id).await?;
    tracing::info!(issues_known = hwms.len(), "loaded per-issue high-water-marks");

    let snapshots = reader::fetch_all_snapshots(&cfg, &args.insight_source_id).await?;
    tracing::info!(snapshots = snapshots.len(), "loaded all issue snapshots");

    let known_issues: Vec<String> = hwms.keys().cloned().collect();
    let last_state = if known_issues.is_empty() {
        HashMap::new()
    } else {
        reader::fetch_last_state_for(&cfg, &args.insight_source_id, &known_issues).await?
    };
    tracing::info!(last_state_issues = last_state.len(), "loaded last state");

    let meta: Arc<HashMap<String, FieldMeta>> = Arc::new(meta);
    let snapshots: Arc<HashMap<String, IssueSnapshot>> = Arc::new(snapshots);
    let last_state: Arc<HashMap<String, HashMap<String, LastState>>> = Arc::new(last_state);

    // --- Phase 2: spawn N writer tasks. Each owns its own Receiver<Batch>. ---
    let mut writer_txs: Vec<mpsc::Sender<Vec<FieldHistoryRecord>>> =
        Vec::with_capacity(args.writers);
    let mut writer_handles = Vec::with_capacity(args.writers);
    for i in 0..args.writers {
        let (tx, mut rx) = mpsc::channel::<Vec<FieldHistoryRecord>>(args.writer_queue_depth);
        let cfg_i = cfg.clone();
        let dry = args.dry_run;
        let timeout_secs = args.insert_timeout_secs;
        let insert_timeout = std::time::Duration::from_secs(timeout_secs);
        writer_txs.push(tx);
        writer_handles.push(tokio::spawn(async move {
            let mut total = 0_usize;
            let mut batches = 0_usize;
            while let Some(batch) = rx.recv().await {
                batches += 1;
                let batch_len = batch.len();
                tracing::info!(writer = i, batch = batches, rows = batch_len, "writer: start INSERT");
                if dry {
                    total += batch_len;
                    continue;
                }
                let t0 = std::time::Instant::now();
                let insert_fut = writer::insert_batch(&cfg_i, batch);
                let n = match tokio::time::timeout(insert_timeout, insert_fut).await {
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => {
                        tracing::error!(
                            writer = i, batch = batches, rows = batch_len,
                            elapsed_ms = t0.elapsed().as_millis() as u64,
                            error = %e,
                            "writer: INSERT failed"
                        );
                        return Err(e);
                    }
                    Err(_) => {
                        tracing::error!(
                            writer = i, batch = batches, rows = batch_len,
                            timeout_secs,
                            "writer: INSERT timed out — likely a CH-side error the client never surfaced \
                             (check system.query_log for ExceptionBeforeStart around now)"
                        );
                        return Err(crate::io::IoError::InsertTimeout(
                            timeout_secs, i, batches, batch_len,
                        ));
                    }
                };
                tracing::info!(
                    writer = i, batch = batches, rows = n,
                    elapsed_ms = t0.elapsed().as_millis() as u64,
                    "writer: INSERT done"
                );
                total += n;
            }
            tracing::info!(writer = i, batches, rows = total, "writer task finished");
            Ok::<usize, crate::io::IoError>(total)
        }));
    }

    // --- Phase 3: reader task streaming cursor → events channel. ---
    let (events_tx, mut events_rx) =
        mpsc::channel::<DeltaEvent>(args.events_channel_capacity);
    let reader_cfg = cfg.clone();
    let reader_source = args.insight_source_id.clone();
    let reader_meta = Arc::clone(&meta);
    let reader_handle = tokio::spawn(async move {
        let mut cursor = reader::open_events_cursor(&reader_cfg, &reader_source)?;
        let mut streamed = 0_usize;
        while let Some(row) = cursor.next().await? {
            streamed += 1;
            if streamed % 50_000 == 0 {
                tracing::info!(streamed, "reader: progress");
            }
            if let Some(ev) = reader::row_to_event(row, &reader_meta) {
                if events_tx.send(ev).await.is_err() {
                    tracing::warn!(streamed, "events receiver dropped — aborting reader");
                    break;
                }
            }
        }
        tracing::info!(streamed, "reader task finished");
        Ok::<usize, crate::io::IoError>(streamed)
    });

    // --- Phase 4: main loop — group per issue, fan out batches round-robin to writers. ---
    let mut current_issue_key: Option<String> = None;
    let mut per_issue_events: Vec<DeltaEvent> = Vec::new();
    let mut out_batch: Vec<FieldHistoryRecord> = Vec::with_capacity(args.batch_size);
    let mut next_writer = 0_usize;

    let mut issues_processed = 0_usize;
    let mut rows_emitted = 0_usize;
    let mut batches_dispatched = 0_usize;

    // Track which issues got events — after the stream is exhausted, we iterate the
    // remaining snapshots and emit synthetic_initial rows for issues that had no events
    // at all (Category C — issues with no changelog, most commonly never-changed creations).
    let mut touched_issues: std::collections::HashSet<String> = std::collections::HashSet::new();

    // When a send fails, it means a writer task exited (usually due to an IoError).
    // Set this flag and break out cleanly; we then drop all senders and await the writer
    // handles — surfacing the FIRST writer's actual error instead of the opaque "dropped".
    let mut writer_send_failed: Option<usize> = None;

    macro_rules! dispatch_full_batches {
        () => {
            while out_batch.len() >= args.batch_size {
                if writer_send_failed.is_some() {
                    break;
                }
                let chunk: Vec<FieldHistoryRecord> =
                    out_batch.drain(..args.batch_size).collect();
                let idx = next_writer;
                next_writer = (next_writer + 1) % args.writers;
                batches_dispatched += 1;
                if writer_txs[idx].send(chunk).await.is_err() {
                    writer_send_failed = Some(idx);
                    break;
                }
            }
        };
    }

    while let Some(ev) = events_rx.recv().await {
        if writer_send_failed.is_some() {
            break;
        }
        if current_issue_key.as_deref() != Some(&ev.id_readable) {
            if let Some(prev_key) = current_issue_key.take() {
                touched_issues.insert(prev_key.clone());
                let n = emit_for_issue(
                    &prev_key,
                    std::mem::take(&mut per_issue_events),
                    &snapshots,
                    &last_state,
                    &meta,
                    &mut out_batch,
                );
                rows_emitted += n;
                issues_processed += 1;
                dispatch_full_batches!();
            }
            current_issue_key = Some(ev.id_readable.clone());
        }
        per_issue_events.push(ev);
    }

    if writer_send_failed.is_none() {
        if let Some(prev_key) = current_issue_key {
            touched_issues.insert(prev_key.clone());
            let n = emit_for_issue(
                &prev_key,
                per_issue_events,
                &snapshots,
                &last_state,
                &meta,
                &mut out_batch,
            );
            rows_emitted += n;
            issues_processed += 1;
        }

        // Category C: issues present in snapshot but with no events in changelog at all.
        let mut snapshot_only_issues = 0_usize;
        for (issue_key, snapshot) in snapshots.iter() {
            if writer_send_failed.is_some() {
                break;
            }
            if touched_issues.contains(issue_key) {
                continue;
            }
            if last_state.contains_key(issue_key) {
                continue;
            }
            let rows = crate::core::process_issue(&meta, snapshot, &[], None);
            if rows.is_empty() {
                continue;
            }
            rows_emitted += rows.len();
            out_batch.extend(rows);
            snapshot_only_issues += 1;
            issues_processed += 1;
            dispatch_full_batches!();
        }
        tracing::info!(
            snapshot_only_issues,
            "bootstrapped issues with no changelog events"
        );

        dispatch_full_batches!();

        if writer_send_failed.is_none() && !out_batch.is_empty() {
            let tail = std::mem::take(&mut out_batch);
            let idx = next_writer;
            batches_dispatched += 1;
            if writer_txs[idx].send(tail).await.is_err() {
                writer_send_failed = Some(idx);
            }
        }
    }

    // Close all writer inputs so tasks complete.
    drop(writer_txs);

    // Await reader and writers — surfacing REAL IoError from any writer that failed.
    let reader_result = reader_handle.await;
    let mut rows_written = 0_usize;
    let mut first_writer_err: Option<(usize, anyhow::Error)> = None;
    for (i, h) in writer_handles.into_iter().enumerate() {
        match h.await {
            Ok(Ok(n)) => rows_written += n,
            Ok(Err(e)) => {
                tracing::error!(writer = i, error = %e, "writer failed");
                if first_writer_err.is_none() {
                    first_writer_err = Some((i, anyhow::anyhow!("writer {i} failed: {e}")));
                }
            }
            Err(e) => {
                tracing::error!(writer = i, error = %e, "writer join error");
                if first_writer_err.is_none() {
                    first_writer_err = Some((i, anyhow::anyhow!("writer {i} panicked: {e}")));
                }
            }
        }
    }

    if let Some((_, e)) = first_writer_err {
        return Err(e);
    }
    if let Some(idx) = writer_send_failed {
        return Err(anyhow::anyhow!(
            "writer {idx} receiver closed but no error surfaced — check writer logs"
        ));
    }

    let streamed = reader_result??;
    tracing::info!(
        issues_processed,
        events_streamed = streamed,
        rows_emitted,
        batches_dispatched,
        rows_written,
        "run complete"
    );

    Ok(())
}

#[cfg(feature = "io")]
fn emit_for_issue(
    issue_key: &str,
    events: Vec<crate::core::types::DeltaEvent>,
    snapshots: &std::collections::HashMap<String, crate::core::types::IssueSnapshot>,
    last_state: &std::collections::HashMap<
        String,
        std::collections::HashMap<String, crate::core::types::LastState>,
    >,
    meta: &std::collections::HashMap<String, crate::core::types::FieldMeta>,
    out: &mut Vec<crate::core::types::FieldHistoryRecord>,
) -> usize {
    let Some(snapshot) = snapshots.get(issue_key) else {
        tracing::warn!(id_readable = %issue_key, "events without snapshot — skipping");
        return 0;
    };
    let existing = last_state.get(issue_key);
    let rows = crate::core::process_issue(meta, snapshot, &events, existing);
    let n = rows.len();
    out.extend(rows);
    n
}
