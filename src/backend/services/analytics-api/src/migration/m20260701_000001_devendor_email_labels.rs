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
//! `m365_emails_read`
//! is intentionally left untouched (its drop belongs to the full de-vendor).
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

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for (metric_key, sublabel) in SUBLABEL_FIXES {
            db.execute_unprepared(&format!(
                "UPDATE metric_catalog SET sublabel = '{sub}' \
                 WHERE tenant_id IS NULL AND metric_key = '{key}'",
                sub = sublabel.replace('\'', "''"),
                key = metric_key.replace('\'', "''"),
            ))
            .await?;
        }
        tracing::info!(
            fixed = SUBLABEL_FIXES.len(),
            "email catalog sublabels de-vendored (#1529)"
        );
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(
            "m20260701_000001_devendor_email_labels is irreversible: \
             restore the prior sublabels from m20260527_000001 manually if needed."
                .to_string(),
        ))
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
}
