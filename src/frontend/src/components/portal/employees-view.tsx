import { Link } from "@tanstack/react-router";
import { Search } from "lucide-react";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { PeopleListItem } from "@/api/identity-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
import {
  Badge,
  Input,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@gears-frontx/ui-kit";
import { personName } from "@/lib/identities/person-display";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { useVisibilityPolicy } from "@/queries/identity-me";
import { useVisibleRoster } from "@/queries/visible-roster";
import { cn } from "@/lib/utils";

interface EmployeeRow {
  personId: string;
  displayName: string;
  jobTitle: string;
  department: string;
  division: string;
  supervisorName: string;
  status: string;
}

function rosterEmployees(
  roster: readonly PeopleListItem[],
  unnamed: string,
): EmployeeRow[] {
  return roster
    .map((person) => ({
      personId: person.person_id,
      displayName: personName(person) ?? unnamed,
      jobTitle: person.attributes.job_title ?? "",
      department: person.attributes.department ?? "",
      division: person.attributes.division ?? "",
      supervisorName: person.manager_name ?? "",
      status: person.attributes.status ?? "",
    }))
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
}

/**
 * Employees directory — every canonical person visible to the viewer,
 * searchable and linked to their Person view.
 */
export function EmployeesView() {
  const { t } = useTranslation();
  const { setZone } = usePortalNavActions();
  const { isFlat } = useVisibilityPolicy();
  const visibleRoster = useVisibleRoster(true);
  const [query, setQuery] = useState("");

  const unnamed = t("identities.person.unnamed");
  const employees = useMemo(
    () => rosterEmployees(visibleRoster.roster, unnamed),
    [visibleRoster.roster, unnamed],
  );
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return employees;
    return employees.filter((e) =>
      [e.displayName, e.jobTitle, e.department, e.division, e.supervisorName]
        .join(" ")
        .toLowerCase()
        .includes(q),
    );
  }, [employees, query]);

  const loading = visibleRoster.isPending;
  const failed = visibleRoster.isError;
  if (loading) return <CenteredSpinner data-testid="page-people-employees" className="min-h-[60vh]" />;
  if (failed)
    return (
      <div data-testid="page-people-employees" className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          onRetry={visibleRoster.retry}
        />
      </div>
    );

  return (
    <div data-testid="page-people-employees" className="flex flex-col gap-3 p-4 md:p-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold tracking-tight">
            {isFlat ? t("people.roster") : t("people.employees")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {filtered.length === employees.length
              ? t("people.people_live", { count: filtered.length })
              : t("people.people_filtered", {
                  shown: filtered.length,
                  total: employees.length,
                })}
          </p>
        </div>
        <div className="relative w-full max-w-xs">
          <Search className="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("people.search_placeholder")}
            className="pl-8"
          />
        </div>
      </div>

      <div className="overflow-x-auto rounded-lg border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("people.name")}</TableHead>
              <TableHead>{t("people.title")}</TableHead>
              {/* A flat organisation fills none of these: no reporting line,
                  and the roster carries no org attributes. */}
              {isFlat ? null : (
                <>
                  <TableHead>{t("people.department")}</TableHead>
                  <TableHead>{t("people.division")}</TableHead>
                  <TableHead>{t("people.manager")}</TableHead>
                </>
              )}
              <TableHead>{t("common.table.status")}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {filtered.map((e) => (
              <TableRow key={e.personId}>
                <TableCell>
                  <Link
                    to="/ic/$person/personal"
                    params={{ person: e.personId }}
                    // Clear the pinned Manage zone so the route-driven Person
                    // zone takes over (same pattern as the rail).
                    onClick={() => setZone(null)}
                    className="font-medium hover:underline"
                  >
                    {e.displayName}
                  </Link>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {e.jobTitle || "—"}
                </TableCell>
                {isFlat ? null : (
                  <>
                    <TableCell className="text-muted-foreground">
                      {e.department || "—"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {e.division || "—"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {e.supervisorName || "—"}
                    </TableCell>
                  </>
                )}
                <TableCell>
                  {e.status ? (
                    <Badge
                      variant="secondary"
                      className={cn(
                        "font-medium",
                        e.status.toLowerCase() === "active"
                          ? "bg-success/15 text-success"
                          : "bg-muted text-muted-foreground",
                      )}
                    >
                      {e.status}
                    </Badge>
                  ) : (
                    "—"
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}
