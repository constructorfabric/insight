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
import { summariseBySubsystem } from "@/lib/gears/summary";
import { useGearRoadmap } from "@/queries/gear-roadmap";

export function GearOverviewScreen() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();

  const rows = useMemo(
    () => summariseBySubsystem(data?.gears ?? []),
    [data],
  );

  if (isPending) return <CenteredSpinner />;
  if (isError) return <p role="alert">{t("gear_roadmap.load_failed")}</p>;

  return (
    <section className="flex flex-col gap-3">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("gear_roadmap.items.subsystem")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.items")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.done")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.done_share")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.spec")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.sdk")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.impl")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.effort")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.remaining")}</TableHead>
            <TableHead>{t("gear_roadmap.overview.unestimated")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((row) => (
            <TableRow key={row.subsystem}>
              <TableCell className="font-medium">{row.subsystem}</TableCell>
              <TableCell>{row.items}</TableCell>
              <TableCell>{row.done}</TableCell>
              <TableCell>{share(row.donePercent)}</TableCell>
              <TableCell>{share(row.specReadiness)}</TableCell>
              <TableCell>{share(row.sdkReadiness)}</TableCell>
              <TableCell>{share(row.implReadiness)}</TableCell>
              <TableCell>{row.effortManDays.toFixed(0)}</TableCell>
              <TableCell>{row.remainingManDays.toFixed(0)}</TableCell>
              <TableCell>{row.unestimated}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </section>
  );
}

function share(value: number | null): string {
  return value === null ? "—" : `${value.toFixed(0)}%`;
}
