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
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { subsystemTone } from "@/lib/gears/subsystem-tone";
import { summariseBySubsystem } from "@/lib/gears/summary";
import { useGearRoadmap } from "@/queries/gear-roadmap";

export function GearSummary() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();

  const rows = useMemo(() => summariseBySubsystem(data?.gears ?? []), [data]);

  if (isPending) return <CenteredSpinner />;
  if (isError) return <p role="alert">{t("gear_roadmap.load_failed")}</p>;

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
                  <Meter value={row.donePercent} />
                </TableCell>
                <TableCell>
                  <Meter value={row.specReadiness} />
                </TableCell>
                <TableCell>
                  <Meter value={row.sdkReadiness} />
                </TableCell>
                <TableCell>
                  <Meter value={row.implReadiness} />
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.effortManDays.toFixed(0)}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.remainingManDays.toFixed(0)}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {row.unestimated === 0 ? (
                    <span className="text-muted-foreground">—</span>
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
                {totals.effort.toFixed(0)}
              </TableCell>
              <TableCell className="text-end tabular-nums">
                {totals.remaining.toFixed(0)}
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

/** A share as a bar plus its number; a dash where nothing carries a value. */
function Meter({ value }: { value: number | null }) {
  if (value === null) {
    return <span className="text-muted-foreground">—</span>;
  }

  return (
    <span className="flex items-center gap-2">
      <span className="h-1.5 w-16 overflow-hidden rounded-full bg-muted">
        <span
          className="block h-full rounded-full bg-primary/70"
          style={{ width: `${Math.min(Math.max(value, 0), 100)}%` }}
        />
      </span>
      <span className="tabular-nums text-xs text-muted-foreground">
        {value.toFixed(0)}%
      </span>
    </span>
  );
}
