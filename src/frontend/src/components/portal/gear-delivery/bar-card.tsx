import { useTranslation } from "react-i18next";

import type { Gear } from "@/api/gear-roadmap-client";
import { AssigneeLinks } from "@/components/portal/gear-delivery/parts";
import { RecordLink } from "@/components/record-link";
import {
  NO_METRIC_VALUE,
  formatMetricNumber,
  formatMetricValue,
} from "@/lib/format";

/** What one scheduled bar stands for, shown on hover. */
export function GearBarCard({
  gear,
  start,
  end,
}: {
  gear: Gear | undefined;
  start: string;
  end: string;
}) {
  const { t } = useTranslation();
  const label = gear?.title ?? "";

  return (
      <div className="flex flex-col gap-2">
        <p className="font-medium">{label}</p>

        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
          <dt className="text-muted-foreground">
            {t("gear_roadmap.gantt.window")}
          </dt>
          <dd className="tabular-nums">
            {start} → {end}
          </dd>

          <dt className="text-muted-foreground">
            {t("gear_roadmap.items.remaining")}
          </dt>
          <dd className="tabular-nums">
            {formatMetricNumber(gear?.remaining_man_days, "integer")}
            {gear?.effort_man_days
              ? ` / ${formatMetricNumber(gear.effort_man_days, "integer")}`
              : ""}
          </dd>

          <dt className="text-muted-foreground">
            {t("gear_roadmap.items.impl")}
          </dt>
          <dd className="tabular-nums">
            {formatMetricValue(gear?.status_percent, "percent")}
          </dd>

          <dt className="text-muted-foreground">
            {t("gear_roadmap.items.milestone")}
          </dt>
          <dd className="tabular-nums">
            <span
              className={
                gear?.placement.kind === "overdue" ? "text-destructive" : undefined
              }
            >
              {gear?.milestone ?? NO_METRIC_VALUE}
            </span>
          </dd>

          <dt className="text-muted-foreground">
            {t("gear_roadmap.items.assignees")}
          </dt>
          <dd>
            <AssigneeLinks
              logins={gear?.assignees ?? []}
              links={gear?.assignee_urls}
            />
          </dd>
        </dl>

        {gear?.issue_url ? (
          <span className="text-xs">
            <RecordLink href={gear.issue_url}>
              {t("gear_roadmap.gantt.open_issue", { number: gear.number })}
            </RecordLink>
          </span>
        ) : null}
      </div>
  );
}
