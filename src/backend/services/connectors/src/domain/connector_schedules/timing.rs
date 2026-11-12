use chrono::{DateTime, Datelike, Days, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use kube::api::DynamicObject;
use serde::Deserialize;

use super::dto::{ScheduleResponse, ScheduleStatus};
use super::validation::{calendar_matches, schedule_bits};
use crate::infra::cluster::Cluster;

const MAX_LOOKAHEAD_DAYS: u64 = 366 * 8;

#[derive(Debug, Deserialize)]
struct ReconcileSchedule {
    #[serde(default)]
    schedules: Vec<String>,
    schedule: Option<String>,
    timezone: Option<String>,
    #[serde(default)]
    suspend: bool,
}

pub(super) async fn enrich(response: &mut ScheduleResponse, cluster: &Cluster) {
    if response.status != Some(ScheduleStatus::Pending)
        || cluster.reconcile_workflow_name.is_empty()
    {
        return;
    }
    let Ok(Some(object)) = cluster.get(&cluster.reconcile_workflow_name).await else {
        return;
    };
    if let Ok(Some((next, interval))) =
        tokio::task::spawn_blocking(move || reconcile_times(&object, Utc::now())).await
    {
        response.next_reconcile_at = Some(next);
        response.reconcile_interval_secs = Some(interval);
    }
}

fn reconcile_times(object: &DynamicObject, now: DateTime<Utc>) -> Option<(DateTime<Utc>, i64)> {
    let spec: ReconcileSchedule = serde_json::from_value(object.data.get("spec")?.clone()).ok()?;
    if spec.suspend {
        return None;
    }
    let timezone: Tz = spec
        .timezone
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("UTC")
        .parse()
        .ok()?;
    let crons = if spec.schedules.is_empty() {
        vec![spec.schedule?]
    } else {
        spec.schedules
    };
    if crons.len() > 10 {
        return None;
    }
    let mut fires = Vec::new();
    for cron in crons {
        fires.extend(next_fires(&cron, timezone, now)?);
    }
    fires.sort_unstable();
    fires.dedup();
    let next = *fires.first()?;
    let interval = (*fires.get(1)? - next).num_seconds();
    Some((next, interval))
}

fn next_fires(cron: &str, timezone: Tz, now: DateTime<Utc>) -> Option<Vec<DateTime<Utc>>> {
    let bits = schedule_bits(cron)?;
    let today = now.with_timezone(&timezone).date_naive();
    let mut fires = Vec::new();
    for offset in 0..MAX_LOOKAHEAD_DAYS {
        let date = today.checked_add_days(Days::new(offset))?;
        if !calendar_matches(
            &bits,
            date.day(),
            date.weekday().num_days_from_sunday(),
            date.month(),
        ) {
            continue;
        }
        fires.extend(day_fires(date, &bits, timezone, now));
        fires.sort_unstable();
        if fires.len() >= 2 {
            fires.truncate(2);
            return Some(fires);
        }
    }
    None
}

fn day_fires(
    date: NaiveDate,
    bits: &[u64; 5],
    timezone: Tz,
    now: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    let mut fires = Vec::new();
    for hour in (0..24).filter(|hour| bits[1] & (1 << hour) != 0) {
        for minute in (0..60).filter(|minute| bits[0] & (1 << minute) != 0) {
            let Some(local) = date.and_hms_opt(hour, minute, 0) else {
                continue;
            };
            let candidates = local_fires(timezone, local);
            fires.extend(candidates.into_iter().flatten().filter(|time| *time > now));
        }
    }
    fires
}

fn local_fires(timezone: Tz, local: NaiveDateTime) -> [Option<DateTime<Utc>>; 2] {
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(time) => [Some(time.with_timezone(&Utc)), None],
        LocalResult::Ambiguous(first, second) => [
            Some(first.with_timezone(&Utc)),
            Some(second.with_timezone(&Utc)),
        ],
        LocalResult::None => [None, None],
    }
}

#[cfg(test)]
mod tests;
