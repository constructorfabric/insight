import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Gear } from "@/api/gear-roadmap-client";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { useGearRoadmap } from "@/queries/gear-roadmap";

export function GearItemsScreen() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();
  const [query, setQuery] = useState("");

  const gears = useMemo(() => filterGears(data?.gears ?? [], query), [
    data,
    query,
  ]);

  if (isPending) return <CenteredSpinner />;
  if (isError) return <p role="alert">{t("gear_roadmap.load_failed")}</p>;

  return (
    <section className="flex flex-col gap-3">
      <Input
        type="search"
        value={query}
        onChange={(event) => setQuery(event.target.value)}
        placeholder={t("gear_roadmap.items.filter_placeholder")}
        aria-label={t("gear_roadmap.items.filter_placeholder")}
        className="max-w-xs"
      />

      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>{t("gear_roadmap.items.gear")}</TableHead>
            <TableHead>{t("gear_roadmap.items.subsystem")}</TableHead>
            <TableHead>{t("gear_roadmap.items.spec")}</TableHead>
            <TableHead>{t("gear_roadmap.items.sdk")}</TableHead>
            <TableHead>{t("gear_roadmap.items.impl")}</TableHead>
            <TableHead>{t("gear_roadmap.items.effort")}</TableHead>
            <TableHead>{t("gear_roadmap.items.remaining")}</TableHead>
            <TableHead>{t("gear_roadmap.items.milestone")}</TableHead>
            <TableHead>{t("gear_roadmap.items.assignees")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {gears.map((gear) => (
            <TableRow key={gear.number}>
              <TableCell className="font-medium">
                {gear.title}
                {gear.commitment === "committed" ? (
                  <Badge variant="secondary" className="ms-2 font-normal">
                    {t("gear_roadmap.committed")}
                  </Badge>
                ) : null}
              </TableCell>
              <TableCell>{gear.subsystem ?? "—"}</TableCell>
              <TableCell>{percent(gear.design_percent)}</TableCell>
              <TableCell>{percent(gear.sdk_percent)}</TableCell>
              <TableCell>{percent(gear.status_percent)}</TableCell>
              <TableCell>{days(gear.effort_man_days)}</TableCell>
              <TableCell>{days(gear.remaining_man_days)}</TableCell>
              <TableCell>
                <span
                  className={
                    gear.placement === "overdue"
                      ? "text-destructive font-medium"
                      : undefined
                  }
                >
                  {gear.milestone ?? "—"}
                </span>
              </TableCell>
              <TableCell>{gear.assignees.join(", ") || "—"}</TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </section>
  );
}

function filterGears(gears: Gear[], query: string): Gear[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") return gears;
  return gears.filter(
    (gear) =>
      gear.title.toLowerCase().includes(needle) ||
      (gear.subsystem ?? "").toLowerCase().includes(needle) ||
      gear.assignees.some((login) => login.toLowerCase().includes(needle)),
  );
}

function percent(value: number | null | undefined): string {
  return typeof value === "number" ? `${value}%` : "—";
}

function days(value: number | null | undefined): string {
  return typeof value === "number" ? value.toFixed(0) : "—";
}
