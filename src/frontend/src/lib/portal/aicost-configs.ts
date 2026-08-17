/**
 * AI & Cost pane items with no dedicated data-backed view yet, and the note
 * each one renders instead. The wording must agree with the item's `readiness`
 * in nav-model — a product gap never reads as a screen we owe, and the reverse
 * (pinned by readiness.test.ts).
 */
export const PANE_ITEM_COMING_SOON: Record<string, string> = {
  "per-tool":
    "Per-tool detail — the tool split is summarised on Overview → By tool; a standalone per-tool drilldown is pending.",
  autofix: "Autofix — no autofix data is collected.",
  "ai-audit": "AI Audit — not built yet.",
  "spend-by-tool":
    "Spend by tool — see Overview → By tool; a dedicated spend breakdown is pending.",
  "cost-by-unit":
    "Cost by unit / user — unit rollup is under “By unit / role”, per-user is on Overview; a combined view is pending.",
  "idle-seats":
    "Idle seats — seat data is collected but not available in this view yet.",
  credits: "Credits burn-down — no credit or quota data is collected.",
  "ai-pricing": "AI pricing settings — not built yet.",
};
