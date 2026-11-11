import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import type { OwnedFilter } from "@/api/custom-client";
import { resolveText } from "@/i18n/localized-text";
import { useDashboardBrowse } from "@/lib/custom/dashboard-browse";
import { cn } from "@/lib/utils";

export function BrowseChips({
  owned,
  tag,
  counted,
}: {
  owned: OwnedFilter | undefined;
  tag: string[] | undefined;
  counted: string[];
}) {
  const { t } = useTranslation();
  const rows = useDashboardBrowse(counted);

  return (
    <nav aria-label={t("nav.browse_dashboards")} className="mb-4 flex flex-wrap gap-2">
      {rows.map((row) => {
        const current = row.owned === owned;
        return (
          <Link
            key={row.owned ?? "all"}
            to="/portal/custom"
            search={{ owned: row.owned, tag }}
            aria-current={current ? "page" : undefined}
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-full border px-3 text-sm transition-colors",
              current
                ? "border-primary bg-primary/10 text-primary"
                : "border-border bg-card text-foreground hover:bg-muted"
            )}
          >
            {row.owned === undefined ? t("nav.items.all") : resolveText(t, row.label)}
            {row.count != null ? " " : null}
            {row.count != null ? (
              <span
                className={cn(
                  "text-xs tabular-nums",
                  current ? "text-primary" : "text-muted-foreground"
                )}
              >
                {row.count}
              </span>
            ) : null}
          </Link>
        );
      })}
    </nav>
  );
}
