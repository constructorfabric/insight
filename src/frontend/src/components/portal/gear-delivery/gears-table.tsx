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
  TableHead,
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
import { UNGROUPED } from "@/lib/gears/roadmap-grid";
import { subsystemTone } from "@/lib/gears/subsystem-tone";

const ALL = "__all__";

export function GearsTable({ roadmap }: { roadmap: GearRoadmap }) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState(ALL);

  const all = useMemo(() => roadmap.gears, [roadmap]);
  const subsystems = useMemo(() => countBySubsystem(all), [all]);
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
        <Select value={chosen} onValueChange={(value) => setChosen(value ?? ALL)}>
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
              <TableHead>{t("gear_roadmap.items.gear")}</TableHead>
              <TableHead>{t("gear_roadmap.items.subsystem")}</TableHead>
              <TableHead className="w-28">
                {t("gear_roadmap.items.spec")}
              </TableHead>
              <TableHead className="w-28">
                {t("gear_roadmap.items.sdk")}
              </TableHead>
              <TableHead className="w-28">
                {t("gear_roadmap.items.impl")}
              </TableHead>
              <TableHead className="text-end">
                {t("gear_roadmap.items.effort")}
              </TableHead>
              <TableHead className="text-end">
                {t("gear_roadmap.items.remaining")}
              </TableHead>
              <TableHead>{t("gear_roadmap.items.milestone")}</TableHead>
              <TableHead>{t("gear_roadmap.items.assignees")}</TableHead>
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

  if (gear.placement.kind === "overdue") {
    return (
      <span className="rounded bg-destructive/10 px-1.5 py-0.5 text-xs font-medium text-destructive tabular-nums">
        {gear.milestone}
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
