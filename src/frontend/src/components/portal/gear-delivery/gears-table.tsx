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
import { UNGROUPED } from "@/lib/gears/roadmap-grid";
import { subsystemTone } from "@/lib/gears/subsystem-tone";
import { useGearRoadmap } from "@/queries/gear-roadmap";

export function GearsTable() {
  const { t } = useTranslation();
  const { data, isPending, isError } = useGearRoadmap();
  const [query, setQuery] = useState("");
  const [chosen, setChosen] = useState<readonly string[]>([]);

  const all = useMemo(() => data?.gears ?? [], [data]);
  const subsystems = useMemo(() => countBySubsystem(all), [all]);
  const gears = useMemo(
    () => filterGears(all, query, chosen),
    [all, query, chosen],
  );

  if (isPending) return <CenteredSpinner />;
  if (isError) return <p role="alert">{t("gear_roadmap.load_failed")}</p>;

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
        <span className="text-xs text-muted-foreground tabular-nums">
          {t("gear_roadmap.items.count", { count: gears.length })}
        </span>
      </div>

      <div className="flex flex-wrap gap-1.5">
        {subsystems.map(([subsystem, count]) => {
          const active = chosen.includes(subsystem);
          return (
            <button
              key={subsystem}
              type="button"
              aria-pressed={active}
              aria-label={t("gear_roadmap.items.subsystem_filter", {
                subsystem,
                count,
              })}
              onClick={() => setChosen(toggle(chosen, subsystem))}
              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs transition ${
                active
                  ? "border-primary bg-primary/10 font-medium"
                  : "border-transparent bg-muted hover:bg-accent"
              }`}
            >
              <span
                className={`size-2 rounded-full ${
                  subsystemTone(subsystem).dot
                }`}
              />
              {subsystem}
              <span className="tabular-nums text-muted-foreground">{count}</span>
            </button>
          );
        })}
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
                    {gear.issue_url ? (
                      <a
                        href={gear.issue_url}
                        target="_blank"
                        rel="noreferrer"
                        className="truncate font-medium underline underline-offset-2"
                        title={gear.title}
                      >
                        {gear.title}
                      </a>
                    ) : (
                      <span className="truncate font-medium" title={gear.title}>
                        {gear.title}
                      </span>
                    )}
                    {gear.commitment === "committed" ? (
                      <Badge variant="secondary" className="font-normal">
                        {t("gear_roadmap.committed")}
                      </Badge>
                    ) : null}
                  </span>
                </TableCell>
                <TableCell>
                  {gear.subsystem === null || gear.subsystem === undefined ? (
                    <span className="text-muted-foreground">—</span>
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
                  <Percent value={gear.design_percent} />
                </TableCell>
                <TableCell>
                  <Percent value={gear.sdk_percent} />
                </TableCell>
                <TableCell>
                  <Percent value={gear.status_percent} />
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {days(gear.effort_man_days)}
                </TableCell>
                <TableCell className="text-end tabular-nums">
                  {days(gear.remaining_man_days)}
                </TableCell>
                <TableCell>
                  <Milestone gear={gear} />
                </TableCell>
                <TableCell className="text-xs text-muted-foreground">
                  <Assignees gear={gear} />
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
    return <span className="text-muted-foreground">—</span>;
  }

  if (gear.placement === "overdue") {
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

function Assignees({ gear }: { gear: Gear }) {
  const links = gear.assignee_urls ?? [];

  if (links.length === 0) {
    return <>{gear.assignees.join(", ") || "—"}</>;
  }

  return (
    <span className="flex flex-wrap gap-x-2">
      {links.map((assignee) =>
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
    </span>
  );
}

function Percent({ value }: { value: number | null | undefined }) {
  if (typeof value !== "number") {
    return <span className="text-muted-foreground">—</span>;
  }

  return (
    <span className="flex items-center gap-2">
      <span className="h-1.5 w-10 overflow-hidden rounded-full bg-muted">
        <span
          className={`block h-full rounded-full ${
            value === 100 ? "bg-emerald-600/70" : "bg-primary/70"
          }`}
          style={{ width: `${value}%` }}
        />
      </span>
      <span className="text-xs tabular-nums text-muted-foreground">
        {value}%
      </span>
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

function toggle(chosen: readonly string[], subsystem: string): string[] {
  return chosen.includes(subsystem)
    ? chosen.filter((entry) => entry !== subsystem)
    : [...chosen, subsystem];
}

/** An empty choice means every subsystem, never none. */
function filterGears(
  gears: Gear[],
  query: string,
  chosen: readonly string[],
): Gear[] {
  const needle = query.trim().toLowerCase();

  return gears.filter((gear) => {
    if (
      chosen.length > 0 &&
      !chosen.includes(gear.subsystem ?? UNGROUPED)
    ) {
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

function days(value: number | null | undefined): string {
  return typeof value === "number" ? value.toFixed(0) : "—";
}
