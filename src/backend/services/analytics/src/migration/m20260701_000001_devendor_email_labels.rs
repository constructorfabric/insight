//! De-vendor the Email metric sublabels in `metric_catalog` (issue #1529,
//! modality slice of #1516 · Gap 1 catalog shape).
//!
//! First, label-only increment: the full de-vendoring (rename `m365_emails_*`
//! → `emails_*`, drop `emails_read`, move onto the shared
//! `collab_person_counter_daily` gold view + FE "Email" grouping) is gated on
//! the scaffold from #1527, which has not landed yet. Until then we only strip
//! the vendor token from the human-facing sublabels — the connector stays
//! carried in `source_tags = ["m365"]`, so no signal is lost.
//!
//! Surgical `UPDATE … SET sublabel` on the product-default rows (`tenant_id` IS
//! NULL) — we touch ONLY the sublabel, never the `metric_key`/`label`/
//! `thresholds`/`source_tags`. Idempotent (re-running sets the same text).
//! `m365_emails_read` is intentionally left untouched (its drop belongs to the
//! full de-vendor).
//!
//! Mirrors the pattern established by `m20260610_000001_fix_ai_label_drift`.

use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// (`metric_key`, de-vendored sublabel). \u{b7} = "·". The vendor ("M365 · ")
/// prefix is dropped; the descriptor + "period total" cadence is preserved.
const SUBLABEL_FIXES: &[(&str, &str)] = &[
    (
        "collab_bullet_rows.m365_emails_sent",
        "Emails sent \u{b7} period total",
    ),
    (
        "collab_bullet_rows.m365_emails_received",
        "Inbox volume \u{b7} period total",
    ),
];

/// Renders the idempotent product-default sublabel `UPDATE` for one metric.
/// Scoped to `tenant_id IS NULL`; single quotes are doubled (belt-and-
/// suspenders — both inputs are compile-time constants). Kept as a pure fn so
/// the SQL shape/scoping/escaping is unit-testable without a live DB (`up()`
/// is a thin `execute_unprepared` loop that only integration tests cover).
fn devendor_sublabel_sql(metric_key: &str, sublabel: &str) -> String {
    format!(
        "UPDATE metric_catalog SET sublabel = '{sub}' \
         WHERE tenant_id IS NULL AND metric_key = '{key}'",
        sub = sublabel.replace('\'', "''"),
        key = metric_key.replace('\'', "''"),
    )
}

/// The error `down()` returns: de-vendoring is one-way — the prior sublabels
/// are restored from `m20260527_000001` if ever needed.
fn irreversible_error() -> DbErr {
    DbErr::Custom(
        "m20260701_000001_devendor_email_labels is irreversible: \
         restore the prior sublabels from m20260527_000001 manually if needed."
            .to_string(),
    )
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (metric_key, sublabel) in SUBLABEL_FIXES {
            db.execute_unprepared(&devendor_sublabel_sql(metric_key, sublabel))
                .await?;
        }
        tracing::info!(
            fixed = SUBLABEL_FIXES.len(),
            "email catalog sublabels de-vendored (#1529)"
        );
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(irreversible_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No vendor token left in the de-vendored sublabels, and both #1529
    /// metrics (sent + received) are covered. `m365_emails_read` is out of
    /// scope for this slice and must NOT be here.
    #[test]
    fn sublabels_are_devendored_and_scoped() {
        for (key, sub) in SUBLABEL_FIXES {
            assert!(
                !sub.contains("M365"),
                "{key}: vendor token still in sublabel"
            );
        }
        let keys: Vec<&str> = SUBLABEL_FIXES.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"collab_bullet_rows.m365_emails_sent"));
        assert!(keys.contains(&"collab_bullet_rows.m365_emails_received"));
        assert!(
            !keys.contains(&"collab_bullet_rows.m365_emails_read"),
            "emails_read drop belongs to the full de-vendor, not this label slice"
        );
    }

    /// The rendered `UPDATE` is product-default scoped, targets the intended
    /// key, carries the de-vendored sublabel (no vendor token), and doubles
    /// single quotes.
    #[test]
    fn devendor_sql_is_scoped_targeted_and_escaped() {
        for (key, sublabel) in SUBLABEL_FIXES {
            let sql = devendor_sublabel_sql(key, sublabel);
            assert!(
                sql.starts_with("UPDATE metric_catalog SET sublabel = '"),
                "{key}: unexpected UPDATE shape: {sql}"
            );
            assert!(
                sql.contains("WHERE tenant_id IS NULL"),
                "{key}: must be scoped to product-default rows"
            );
            assert!(
                sql.contains(&format!("metric_key = '{key}'")),
                "{key}: must target its own metric_key"
            );
            assert!(sql.contains(sublabel), "{key}: rendered sublabel missing");
            assert!(!sql.contains("M365"), "{key}: vendor token leaked into SQL");
        }

        // Single quotes in either value are doubled, never emitted raw.
        let sql = devendor_sublabel_sql("a'b", "c'd");
        assert!(
            sql.contains("metric_key = 'a''b'"),
            "key not escaped: {sql}"
        );
        assert!(
            sql.contains("sublabel = 'c''d'"),
            "sublabel not escaped: {sql}"
        );
    }

    /// `down()` is one-way: it returns a `DbErr::Custom` that names the
    /// restore path so an operator isn't left guessing.
    #[test]
    fn down_is_irreversible_with_restore_hint() {
        match irreversible_error() {
            DbErr::Custom(msg) => {
                assert!(msg.contains("irreversible"), "message: {msg}");
                assert!(
                    msg.contains("m20260527_000001"),
                    "must name restore source: {msg}"
                );
            }
            other => panic!("expected DbErr::Custom, got {other:?}"),
        }
    }
}
