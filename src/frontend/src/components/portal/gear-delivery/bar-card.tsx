import { useTranslation } from "react-i18next";

import type { Gear } from "@/api/gear-roadmap-client";

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
            {gear?.remaining_man_days?.toFixed(0) ?? "—"}
            {gear?.effort_man_days
              ? ` / ${gear.effort_man_days.toFixed(0)}`
              : ""}
          </dd>

          <dt className="text-muted-foreground">
            {t("gear_roadmap.items.impl")}
          </dt>
          <dd className="tabular-nums">
            {typeof gear?.status_percent === "number"
              ? `${gear.status_percent}%`
              : "—"}
          </dd>

          <dt className="text-muted-foreground">
            {t("gear_roadmap.items.milestone")}
          </dt>
          <dd className="tabular-nums">
            <span
              className={
                gear?.placement === "overdue" ? "text-destructive" : undefined
              }
            >
              {gear?.milestone ?? "—"}
            </span>
          </dd>

          <dt className="text-muted-foreground">
            {t("gear_roadmap.items.assignees")}
          </dt>
          <dd className="flex flex-wrap gap-x-2">
            {(gear?.assignee_urls ?? []).length === 0
              ? (gear?.assignees.join(", ") || "—")
              : gear?.assignee_urls?.map((assignee) =>
                  assignee.url ? (
                    <a
                      key={assignee.login}
                      href={assignee.url}
                      target="_blank"
                      rel="noreferrer"
                      className="underline underline-offset-2"
                    >
                      {assignee.login}
                    </a>
                  ) : (
                    <span key={assignee.login}>{assignee.login}</span>
                  ),
                )}
          </dd>
        </dl>

        {gear?.issue_url ? (
          <a
            href={gear.issue_url}
            target="_blank"
            rel="noreferrer"
            className="text-xs underline underline-offset-2"
          >
            {t("gear_roadmap.gantt.open_issue", { number: gear.number })}
          </a>
        ) : null}
      </div>
  );
}
