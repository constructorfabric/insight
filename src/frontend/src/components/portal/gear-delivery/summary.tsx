import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { GearRoadmap } from "@/api/gear-roadmap-client";
import { ShareBar } from "@/components/portal/gear-delivery/parts";
import { NO_METRIC_VALUE, formatMetricNumber } from "@/lib/format";
import { subsystemTone } from "@/lib/gears/subsystem-tone";
import { summariseBySubsystem } from "@/lib/gears/summary";

export function GearSummary({ roadmap }: { roadmap: GearRoadmap }) {
  const { t } = useTranslation();

  const rows = useMemo(() => summariseBySubsystem(roadmap.gears), [roadmap]);


  const totals = rows.reduce(
    (sum, row) => ({
      items: sum.items + row.items,
      done: sum.done + row.done,
      effort: sum.effort + row.effortManDays,
      remaining: sum.remaining + row.remainingManDays,
      unestimated: sum.unestimated + row.unestimated,
    }),
    { items: 0, done: 0, effort: 0, remaining: 0, unestimated: 0 },
  );

  return (
    <section className="flex flex-col gap-3">
      <div className="overflow-x-auto rounded-lg border bg-card">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/40 hover:bg-muted/40">
              <TableHead>{t("gear_roadmap.items.subsystem")}</TableHead>
              <TableHead className="text-end">
                {t("gear_roadmap.overview.items")}
              </TableHead>
              <TableHead className="text-end">
                {t("gear_roadmap.overview.done")}
              </TableHead>
              <TableHead className="w-40">
                {t("gear_roadmap.overview.done_share")}
              </TableHead>
              <TableHead className="w-32">
                {t("gear_roadmap.overview.spec")}
              </TableHead>
              <TableHead className="w-32">
                {t("gear_roadmap.overview.sdk")}
              </TableHead>
              <TableHead className="w-32">
                {t("gear_roadmap.overview.impl")}
              </TableHead>
              <TableHead className="text-end">
                {t("gear_roadmap.overview.effort")}
              </TableHead>
              <TableHead className="text-end">
                {t("gear_roadmap.overview.remaining")}
              </TableHead>
              <TableHead className="text-end">
                {t("gear_roadmap.overview.unestimated")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow key={row.subsystem}>
                <TableCell>
                  <span
                    className={`rounded px-1.5 py-0.5 text-xs font-medium ${
                      subsystemTone(row.subsystem).chip
                    }`}
                  >
                    {row.subsystem}
                  </span>
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.items}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.done}
                </TableCell>
                <TableCell>
                  <ShareBar value={row.donePercent} width="w-16" />
                </TableCell>
                <TableCell>
                  <ShareBar value={row.specReadiness} width="w-16" />
                </TableCell>
                <TableCell>
                  <ShareBar value={row.sdkReadiness} width="w-16" />
                </TableCell>
                <TableCell>
                  <ShareBar value={row.implReadiness} width="w-16" />
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {formatMetricNumber(row.effortManDays, "integer")}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {formatMetricNumber(row.remainingManDays, "integer")}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.unestimated === 0 ? (
                    <span className="text-muted-foreground">{NO_METRIC_VALUE}</span>
                  ) : (
                    row.unestimated
                  )}
                </TableCell>
              </TableRow>
            ))}
            <TableRow className="border-t-2 font-medium hover:bg-transparent">
              <TableCell>{t("gear_roadmap.overview.total")}</TableCell>
              <TableCell className="text-end tabular-nums">
                {totals.items}
              </TableCell>
              <TableCell className="text-end tabular-nums">
                {totals.done}
              </TableCell>
              <TableCell colSpan={4} />
              <TableCell className="text-end tabular-nums">
                {formatMetricNumber(totals.effort, "integer")}
              </TableCell>
              <TableCell className="text-end tabular-nums">
                {formatMetricNumber(totals.remaining, "integer")}
              </TableCell>
              <TableCell className="text-end tabular-nums">
                {totals.unestimated}
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </div>
    </section>
  );
}
