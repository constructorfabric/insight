import { Link } from "@tanstack/react-router";
import { Search } from "lucide-react";
import { useMemo, useState } from "react";

import type { PeopleListItem } from "@/api/identity-client";
import { CenteredSpinner } from "@/components/widgets/centered-spinner";
import { ComingSoon } from "@/components/widgets/coming-soon";
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
import { personName } from "@/lib/identities/person-display";
import { usePortalNavActions } from "@/lib/portal/portal-nav";
import { useVisibilityPolicy } from "@/queries/identity-me";
import { useVisibleRoster } from "@/queries/visible-roster";
import { cn } from "@/lib/utils";

// Mirrors the rail: a person with neither display name nor email is still a row.
const UNNAMED_PERSON = "Unnamed person";

interface EmployeeRow {
  personId: string;
  displayName: string;
  jobTitle: string;
  department: string;
  division: string;
  supervisorName: string;
  status: string;
}

function rosterEmployees(roster: readonly PeopleListItem[]): EmployeeRow[] {
  const byId = new Map(
    roster.map((person) => [person.person_id.toLowerCase(), person]),
  );
  return roster
    .map((person) => ({
      personId: person.person_id,
      displayName: personName(person) ?? UNNAMED_PERSON,
      jobTitle: person.attributes.job_title ?? "",
      department: person.attributes.department ?? "",
      division: person.attributes.division ?? "",
      supervisorName: person.manager_person_id
        ? personName(byId.get(person.manager_person_id.toLowerCase()) ?? {}) ?? ""
        : "",
      status: person.attributes.status ?? "",
    }))
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
}

/**
 * Employees directory — every canonical person visible to the viewer,
 * searchable and linked to their Person view.
 */
export function EmployeesView() {
  const { setZone } = usePortalNavActions();
  const { isFlat } = useVisibilityPolicy();
  const visibleRoster = useVisibleRoster(true);
  const [query, setQuery] = useState("");

  const employees = useMemo(
    () => rosterEmployees(visibleRoster.roster),
    [visibleRoster.roster],
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
  if (loading) return <CenteredSpinner className="min-h-[60vh]" />;
  if (failed)
    return (
      <div className="mx-auto w-full max-w-md p-8">
        <ComingSoon
          variant="card"
          state="error"
          onRetry={visibleRoster.retry}
        />
      </div>
    );

  return (
    <div className="flex flex-col gap-3 p-4 md:p-6">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold tracking-tight">
            {isFlat ? "Roster" : "Employees"}
          </h1>
          <p className="text-sm text-muted-foreground">
            {filtered.length}
            {filtered.length !== employees.length ? ` of ${employees.length}` : ""}{" "}
            people · live from the identity directory
          </p>
        </div>
        <div className="relative w-full max-w-xs">
          <Search className="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search name, title, department…"
            className="pl-8"
          />
        </div>
      </div>

      <div className="overflow-x-auto rounded-lg border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Title</TableHead>
              {/* A flat organisation fills none of these: no reporting line,
                  and the roster carries no org attributes. */}
              {isFlat ? null : (
                <>
                  <TableHead>Department</TableHead>
                  <TableHead>Division</TableHead>
                  <TableHead>Manager</TableHead>
                </>
              )}
              <TableHead>Status</TableHead>
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
