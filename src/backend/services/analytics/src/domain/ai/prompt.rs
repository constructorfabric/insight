//! Turning what people wrote into the two messages Claude is sent.

use std::fmt::Write as _;

use crate::migration::ai_assist_schema;

/// The instructions a tenant gets until an admin writes its own.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You explain a chart to the person looking at it. The payload carries every line \
the chart draws with its buckets, and says whether the reading is one person's \
or an organisation-wide rollup.

Answer in exactly three paragraphs separated by a blank line. No headings, no \
bullet lists.

First paragraph, one sentence: what this view covers.

Second paragraph: what the lines actually do. Name a real movement — a rise, a \
fall, a step, a divergence between lines — and when it happened. If the lines \
only wobble around a flat level, say that plainly and do not manufacture a \
pattern out of ordinary variation.

Third paragraph: why it might be. Two or three candidate explanations, most \
plausible first, then the single check that would tell them apart. Where a \
context note above explains the movement, name that note. Reach for the \
ordinary cause first — a holiday, a release freeze, joiners or leavers, a \
changed metric definition, a final bucket that is still filling — before \
anything clever.

Do not re-read numbers the chart already shows; cite one only when it carries \
the argument. Describe systems, never judge a person. No praise, no coaching, \
no recommendations nobody asked for. Around 120 words.";

/// One thing somebody wrote down, ready to be read into a prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub body: String,
}

/// Compose the system message: the instructions, then the organisation's
/// context, then the caller's own.
///
/// Entries are read oldest first and stop at the character budget. A cut is
/// stated in the prompt rather than made silently, so an answer missing a note
/// can be explained by the text the model actually received.
pub fn build_system_prompt(base: &str, tenant: &[Entry], person: &[Entry]) -> String {
    let mut out = String::from(base.trim());
    let mut budget = ai_assist_schema::MAX_CONTEXT_CHARS;
    let mut dropped = 0_usize;

    for (heading, entries) in [
        ("Organisation context", tenant),
        ("The reader's own context", person),
    ] {
        let mut wrote_heading = false;
        for entry in entries {
            let block = format!("\n\n### {}\n{}", entry.title.trim(), entry.body.trim());
            let cost = block.chars().count();
            if cost > budget {
                dropped += 1;
                continue;
            }
            if !wrote_heading {
                let _ = write!(out, "\n\n## {heading}");
                wrote_heading = true;
            }
            out.push_str(&block);
            budget -= cost;
        }
    }

    if dropped > 0 {
        let _ = write!(
            out,
            "\n\n## Note\n{dropped} further context entries did not fit in this prompt and were \
             left out. Say so if the reader's question seems to depend on them."
        );
    }

    out
}

/// Whose reading this is.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotScope {
    /// One person's own figure.
    #[default]
    Person,
    /// A rollup over a group of people.
    Organisation,
}

/// One line of a chart, as it is drawn.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct SnapshotSeries {
    pub label: String,
    /// Readings per bucket, oldest first; a gap is null.
    pub points: Vec<Option<f64>>,
}

/// The reading as the viewer sees it, handed to the model as the thing to explain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct MetricSnapshot {
    /// Catalog key, e.g. `tasks.closed`.
    pub metric_key: String,
    /// The label the tile shows.
    pub label: String,
    /// The formatted value the tile shows.
    pub value: String,
    /// What the period is called on screen, e.g. `month`.
    pub period: String,
    /// Inclusive start of the window, `YYYY-MM-DD`.
    pub since: String,
    /// Inclusive end of the window, `YYYY-MM-DD`.
    pub until: String,
    /// The tile's own change line, empty when it has none.
    #[serde(default)]
    pub delta: String,
    /// The tile's peer-comparison line, empty when it has none.
    #[serde(default)]
    pub peer: String,
    /// The catalog's description of the metric, empty when it has none.
    #[serde(default)]
    pub help: String,
    /// The sparkline's readings, oldest first.
    #[serde(default)]
    pub trend: Vec<Option<f64>>,
    /// Whose reading this is. Absent means one person's.
    #[serde(default)]
    pub scope: SnapshotScope,
    /// Bucket start dates every series is indexed by, oldest first.
    #[serde(default)]
    pub bucket_starts: Vec<String>,
    /// The chart's lines, when the reading is a chart rather than a tile.
    #[serde(default)]
    pub series: Vec<SnapshotSeries>,
}

/// Longest any one snapshot field may be once it reaches the prompt.
const MAX_FIELD_CHARS: usize = 300;

/// Most readings one line can contribute.
const MAX_TREND_POINTS: usize = 64;

/// Most lines one chart may hand over.
const MAX_SERIES: usize = 8;

/// The user message: the snapshot as JSON, with a one-line instruction so an
/// empty context still produces an answer about this metric.
///
/// Every field is clipped first. The snapshot arrives from the browser, so
/// without a bound one request could hand the upstream a megabyte of text on
/// the caller's own key.
pub fn snapshot_message(snapshot: &MetricSnapshot) -> String {
    let body = serde_json::to_string_pretty(&clip_snapshot(snapshot))
        .unwrap_or_else(|_| "{\"error\":\"snapshot could not be encoded\"}".to_owned());

    format!("Explain this metric reading:\n\n{body}")
}

/// Dates the model can trust, or none at all.
///
/// The axis is only meaningful when every line has one reading per date. A
/// mismatched pair would have the model name the wrong bucket with total
/// confidence, so the dates are dropped rather than sent misaligned — the
/// prompt already copes with a snapshot that carries none.
fn aligned_axis(dates: Vec<String>, series: &[SnapshotSeries]) -> Vec<String> {
    if dates.is_empty() {
        return dates;
    }
    if series.iter().any(|s| s.points.len() != dates.len()) {
        return Vec::new();
    }
    dates
}

fn clip_snapshot(snapshot: &MetricSnapshot) -> MetricSnapshot {
    let series: Vec<SnapshotSeries> = snapshot
        .series
        .iter()
        .take(MAX_SERIES)
        .map(|s| SnapshotSeries {
            label: clip(&s.label),
            points: s.points.iter().take(MAX_TREND_POINTS).copied().collect(),
        })
        .collect();

    let bucket_starts = aligned_axis(
        snapshot
            .bucket_starts
            .iter()
            .take(MAX_TREND_POINTS)
            .map(|d| clip(d))
            .collect(),
        &series,
    );

    MetricSnapshot {
        metric_key: clip(&snapshot.metric_key),
        label: clip(&snapshot.label),
        value: clip(&snapshot.value),
        period: clip(&snapshot.period),
        since: clip(&snapshot.since),
        until: clip(&snapshot.until),
        delta: clip(&snapshot.delta),
        peer: clip(&snapshot.peer),
        help: clip(&snapshot.help),
        trend: snapshot
            .trend
            .iter()
            .take(MAX_TREND_POINTS)
            .copied()
            .collect(),
        scope: snapshot.scope,
        bucket_starts,
        series,
    }
}

fn clip(value: &str) -> String {
    value.chars().take(MAX_FIELD_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, body: &str) -> Entry {
        Entry {
            title: title.to_owned(),
            body: body.to_owned(),
        }
    }

    fn snapshot() -> MetricSnapshot {
        MetricSnapshot {
            metric_key: "tasks.closed".to_owned(),
            label: "Tasks closed".to_owned(),
            value: "34".to_owned(),
            period: "month".to_owned(),
            since: "2026-08-01".to_owned(),
            until: "2026-08-22".to_owned(),
            delta: "+6 since last month".to_owned(),
            peer: "Team median 27".to_owned(),
            help: "Tasks moved to a closed state in the window.".to_owned(),
            trend: vec![Some(1.0), None, Some(3.0)],
            scope: SnapshotScope::Person,
            bucket_starts: Vec::new(),
            series: Vec::new(),
        }
    }

    #[test]
    fn system_prompt_orders_base_then_tenant_then_person() -> Result<(), String> {
        let out = build_system_prompt(
            "BASE",
            &[entry("Org note", "org body")],
            &[entry("My note", "my body")],
        );

        let at = |needle: &str| out.find(needle).ok_or_else(|| format!("missing {needle}"));
        assert!(at("BASE")? < at("Org note")?, "unexpected order: {out}");
        assert!(at("Org note")? < at("My note")?, "unexpected order: {out}");
        Ok(())
    }

    #[test]
    fn system_prompt_omits_a_heading_for_an_empty_scope() {
        let out = build_system_prompt("BASE", &[], &[entry("My note", "my body")]);

        assert!(!out.contains("Organisation context"), "{out}");
        assert!(out.contains("The reader's own context"), "{out}");
    }

    #[test]
    fn system_prompt_states_that_it_left_entries_out() {
        let huge = "x".repeat(ai_assist_schema::MAX_CONTEXT_CHARS);
        let out = build_system_prompt(
            "BASE",
            &[entry("Fits", "small"), entry("Too big", &huge)],
            &[],
        );

        assert!(out.contains("Fits"), "{out}");
        assert!(!out.contains(&huge), "the oversized entry was included");
        assert!(out.contains("did not fit"), "{out}");
    }

    #[test]
    fn system_prompt_without_context_is_just_the_instructions() {
        assert_eq!(build_system_prompt("  BASE  ", &[], &[]), "BASE");
    }

    #[test]
    fn a_snapshot_field_cannot_grow_the_prompt_without_bound() -> Result<(), serde_json::Error> {
        let mut huge = snapshot();
        huge.label = "x".repeat(10_000);
        huge.trend = vec![Some(1.0); 500];

        let message = snapshot_message(&huge);
        let json = message
            .split_once("\n\n")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(json)?;

        assert_eq!(parsed["label"].as_str().map(str::len), Some(300));
        assert_eq!(parsed["trend"].as_array().map(Vec::len), Some(64));
        Ok(())
    }

    #[test]
    fn a_chart_hands_over_a_bounded_number_of_lines() -> Result<(), serde_json::Error> {
        let mut chart = snapshot();
        chart.scope = SnapshotScope::Organisation;
        chart.series = (0..20)
            .map(|i| SnapshotSeries {
                label: format!("series {i}"),
                points: vec![Some(1.0); 500],
            })
            .collect();

        let message = snapshot_message(&chart);
        let json = message
            .split_once("\n\n")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(json)?;

        assert_eq!(parsed["scope"], "organisation");
        assert_eq!(parsed["series"].as_array().map(Vec::len), Some(8));
        assert_eq!(
            parsed["series"][0]["points"].as_array().map(Vec::len),
            Some(64)
        );
        Ok(())
    }

    #[test]
    fn a_chart_carries_the_dates_its_lines_are_indexed_by() -> Result<(), serde_json::Error> {
        let mut chart = snapshot();
        chart.bucket_starts = (0..100).map(|i| format!("2026-08-{i:02}")).collect();

        let message = snapshot_message(&chart);
        let json = message
            .split_once("\n\n")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(json)?;

        assert_eq!(parsed["bucket_starts"].as_array().map(Vec::len), Some(64));
        assert_eq!(parsed["bucket_starts"][0], "2026-08-00");
        Ok(())
    }

    #[test]
    fn dates_that_do_not_index_the_lines_are_dropped() -> Result<(), serde_json::Error> {
        let mut chart = snapshot();
        chart.bucket_starts = vec!["2026-08-01".to_owned(), "2026-08-02".to_owned()];
        chart.series = vec![SnapshotSeries {
            label: "PRs merged".to_owned(),
            points: vec![Some(1.0)],
        }];

        let message = snapshot_message(&chart);
        let json = message
            .split_once("\n\n")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(json)?;

        assert_eq!(
            parsed["bucket_starts"].as_array().map(Vec::len),
            Some(0),
            "a two-date axis cannot describe a one-point line"
        );
        Ok(())
    }

    #[test]
    fn dates_that_index_every_line_are_kept() -> Result<(), serde_json::Error> {
        let mut chart = snapshot();
        chart.bucket_starts = vec!["2026-08-01".to_owned(), "2026-08-02".to_owned()];
        chart.series = vec![SnapshotSeries {
            label: "PRs merged".to_owned(),
            points: vec![Some(1.0), None],
        }];

        let message = snapshot_message(&chart);
        let json = message
            .split_once("\n\n")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(json)?;

        assert_eq!(parsed["bucket_starts"].as_array().map(Vec::len), Some(2));
        Ok(())
    }

    #[test]
    fn snapshot_message_carries_valid_json() -> Result<(), serde_json::Error> {
        let message = snapshot_message(&snapshot());
        let json = message
            .split_once("\n\n")
            .map(|(_, rest)| rest)
            .unwrap_or_default();

        let parsed: serde_json::Value = serde_json::from_str(json)?;
        assert_eq!(parsed["metric_key"], "tasks.closed");
        assert_eq!(parsed["trend"][1], serde_json::Value::Null);
        Ok(())
    }
}
