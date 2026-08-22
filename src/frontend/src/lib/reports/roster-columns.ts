import type { ReportPerson } from "@/lib/identities/report-person";

export type { ReportPerson } from "@/lib/identities/report-person";

/**
 * A person and their manager each get a name AND a key.
 *
 * Display names collide, and a pivot grouped on one silently merges two
 * people who share it — the same reason cohorts key `manager` on the
 * supervisor's email rather than their display name.
 */
interface ReportPersonColumn {
  header: string;
  of: (person: ReportPerson) => string;
  always?: boolean;
}

const PERSON_COLUMNS: readonly ReportPersonColumn[] = [
  { header: "Person", of: (p) => p.name, always: true },
  { header: "Email", of: (p) => p.email },
  { header: "Division", of: (p) => p.division },
  { header: "Department", of: (p) => p.department },
  { header: "Job title", of: (p) => p.jobTitle },
  { header: "Manager", of: (p) => p.managerName },
  { header: "Manager email", of: (p) => p.managerEmail },
  { header: "Status", of: (p) => p.status },
];

export function reportPersonColumns(
  people: readonly ReportPerson[],
): readonly ReportPersonColumn[] {
  return PERSON_COLUMNS.filter(
    (column) =>
      column.always || people.some((person) => column.of(person).trim() !== ""),
  );
}
