import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Gear } from "@/api/gear-roadmap-client";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { GearRoadmap } from "@/api/gear-roadmap-client";
import {
  AssigneeLinks,
  ShareBar,
} from "@/components/portal/gear-delivery/parts";
import { RecordLink } from "@/components/record-link";
import { NO_METRIC_VALUE, formatMetricNumber } from "@/lib/format";
import { SortableHead } from "@/components/portal/gear-delivery/sortable-head";
import { UNGROUPED } from "@/lib/gears/roadmap-grid";
import {
  usePortalGearOrder,
  usePortalNavActions,
  usePortalSubsystem,
} from "@/lib/portal/portal-nav";
import type { SortState } from "@/lib/gears/sort";
import { subsystemTone } from "@/lib/gears/subsystem-tone";

const ALL = "__all__";

export function GearsTable({ roadmap }: { roadmap: GearRoadmap }) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const { setSubsystem, setGearOrder } = usePortalNavActions();
  const chosen = usePortalSubsystem() || ALL;
  const order = usePortalGearOrder();
  const sort: SortState<GearColumn> | null = order.sort
    ? {
        key: order.sort as GearColumn,
        direction: order.direction === "desc" ? "desc" : "asc",
      }
    : null;

  const all = useMemo(() => roadmap.gears, [roadmap]);
  const subsystems = useMemo(() => countBySubsystem(all), [all]);
  // Order comes from the server, so the table filters what it was handed and
  // never re-sequences it.
  const gears = useMemo(
    () => filterGears(all, query, chosen),
    [all, query, chosen],
  );


  return (
    <section className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <Input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("gear_roadmap.items.filter_placeholder")}
          aria-label={t("gear_roadmap.items.filter_placeholder")}
          className="max-w-xs"
        />
        <Select
          value={chosen}
          onValueChange={(value) =>
            setSubsystem(value && value !== ALL ? value : null)
          }
        >
          <SelectTrigger size="sm" aria-label="Subsystem" className="w-44">
            <SelectValue>
              {chosen === ALL ? t("gear_roadmap.items.all_subsystems") : chosen}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ALL}>
              {t("gear_roadmap.items.all_subsystems")}
            </SelectItem>
            {subsystems.map(([subsystem, count]) => (
              <SelectItem
                key={subsystem}
                value={subsystem}
                aria-label={`${subsystem} — ${count} gears`}
              >
                <span className="flex items-center gap-2">
                  <span
                    className={`size-2 rounded-full ${subsystemTone(subsystem).dot}`}
                  />
                  {subsystem}
                  <span className="tabular-nums text-muted-foreground">
                    {count}
                  </span>
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <span className="text-xs text-muted-foreground tabular-nums">
          {t("gear_roadmap.items.count", { count: gears.length })}
        </span>
      </div>


      <div className="overflow-x-auto rounded-lg border bg-card">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/40 hover:bg-muted/40">
              {GEAR_COLUMNS.map((column) => (
                <SortableHead
                  key={column.key}
                  column={column.key}
                  label={t(`gear_roadmap.items.${column.key}`)}
                  sort={sort}
                  onSort={(next) =>
                    setGearOrder(next?.key ?? null, next?.direction ?? null)
                  }
                  numeric={column.numeric}
                  className={column.width}
                />
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {gears.map((gear) => (
              <TableRow key={gear.number}>
                <TableCell className="max-w-96">
                  <span className="flex items-center gap-2">
                    <span className="truncate font-medium" title={gear.title}>
                      <RecordLink href={gear.issue_url ?? undefined}>
                        {gear.title}
                      </RecordLink>
                    </span>
                    {gear.commitment === "committed" ? (
                      <Badge variant="secondary" className="font-normal">
                        {t("gear_roadmap.committed")}
                      </Badge>
                    ) : null}
                  </span>
                </TableCell>
                <TableCell>
                  {gear.subsystem === null || gear.subsystem === undefined ? (
                    <span className="text-muted-foreground">{NO_METRIC_VALUE}</span>
                  ) : (
                    <span
                      className={`rounded px-1.5 py-0.5 text-xs font-medium ${
                        subsystemTone(gear.subsystem).chip
                      }`}
                    >
                      {gear.subsystem}
                    </span>
                  )}
                </TableCell>
                <TableCell>
                  <ShareBar value={gear.design_percent} width="w-10" />
                </TableCell>
                <TableCell>
                  <ShareBar value={gear.sdk_percent} width="w-10" />
                </TableCell>
                <TableCell>
                  <ShareBar value={gear.status_percent} width="w-10" />
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {formatMetricNumber(gear.effort_man_days, "integer")}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {formatMetricNumber(gear.remaining_man_days, "integer")}
                </TableCell>
                <TableCell>
                  <Milestone gear={gear} />
                </TableCell>
                <TableCell>
                  <Forecast gear={gear} />
                </TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  <AssigneeLinks logins={gear.assignees} links={gear.assignee_urls} />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </section>
  );
}

function Milestone({ gear }: { gear: Gear }) {
  if (gear.milestone === null || gear.milestone === undefined) {
    return <span className="text-muted-foreground">{NO_METRIC_VALUE}</span>;
  }

  if (gear.placement.kind === "delivered") {
    return (
      <span className="text-xs tabular-nums text-muted-foreground line-through">
        {gear.milestone}
      </span>
    );
  }

  if (gear.placement.kind === "overdue") {
    return (
      <span className="rounded bg-destructive/10 px-1.5 py-0.5 text-xs font-medium text-destructive tabular-nums">
        {gear.milestone} · {gear.placement.days}d late
      </span>
    );
  }

  return (
    <span className="text-xs tabular-nums text-muted-foreground">
      {gear.milestone}
    </span>
  );
}

/** Subsystems present on the board, most gears first. */
function countBySubsystem(gears: Gear[]): [string, number][] {
  const counts = new Map<string, number>();

  for (const gear of gears) {
    const key = gear.subsystem ?? UNGROUPED;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }

  return [...counts.entries()].sort(
    (left, right) => right[1] - left[1] || left[0].localeCompare(right[0]),
  );
}

type GearColumn =
  | "gear"
  | "subsystem"
  | "spec"
  | "sdk"
  | "impl"
  | "effort"
  | "remaining"
  | "milestone"
  | "forecast"
  | "assignees";

const GEAR_COLUMNS: {
  key: GearColumn;
  numeric?: boolean;
  width?: string;
}[] = [
  { key: "gear" },
  { key: "subsystem" },
  { key: "spec", width: "w-28" },
  { key: "sdk", width: "w-28" },
  { key: "impl", width: "w-28" },
  { key: "effort", numeric: true },
  { key: "remaining", numeric: true },
  { key: "milestone" },
  { key: "forecast" },
  { key: "assignees" },
];

function filterGears(gears: Gear[], query: string, chosen: string): Gear[] {
  const needle = query.trim().toLowerCase();

  return gears.filter((gear) => {
    if (chosen !== ALL && (gear.subsystem ?? UNGROUPED) !== chosen) {
      return false;
    }

    if (needle === "") return true;

    return (
      gear.title.toLowerCase().includes(needle) ||
      (gear.subsystem ?? "").toLowerCase().includes(needle) ||
      gear.assignees.some((login) => login.toLowerCase().includes(needle))
    );
  });
}

/**
 * When the schedule lands the gear, against what the board promised. Late is
 * the point of the column, so a forecast past its milestone is marked and the
 * rest stay quiet.
 */
function Forecast({ gear }: { gear: Gear }) {
  if (gear.forecast == null) {
    return <span className="text-muted-foreground">{NO_METRIC_VALUE}</span>;
  }

  const late =
    gear.milestone != null &&
    /^\d{4}-\d{2}$/.test(gear.milestone) &&
    gear.forecast > gear.milestone;

  return (
    <span
      className={`text-xs tabular-nums ${
        late
          ? "rounded bg-destructive/10 px-1.5 py-0.5 font-medium text-destructive"
          : "text-muted-foreground"
      }`}
    >
      {gear.forecast}
    </span>
  );
}
