import { normalizePersonId } from "@/lib/metrics/entity";
import { personDisplayName } from "@/lib/identities/person-display";
import type { IdentityPerson } from "@/types/insight";

export interface ReportPerson {
  entityId: string;
  name: string;
  email: string;
  division: string;
  department: string;
  jobTitle: string;
  managerName: string;
  managerEmail: string;
  status: string;
}

/**
 * A person and their manager each get a name AND a key.
 *
 * Display names collide, and a pivot grouped on one silently merges two
 * people who share it — the same reason cohorts key `manager` on the
 * supervisor's email rather than their display name.
 */
export const PERSON_COLUMNS: ReadonlyArray<{
  header: string;
  of: (person: ReportPerson) => string;
}> = [
  { header: "Person", of: (p) => p.name },
  { header: "Email", of: (p) => p.email },
  { header: "Division", of: (p) => p.division },
  { header: "Department", of: (p) => p.department },
  { header: "Job title", of: (p) => p.jobTitle },
  { header: "Manager", of: (p) => p.managerName },
  { header: "Manager email", of: (p) => p.managerEmail },
  { header: "Status", of: (p) => p.status },
];

export function collectReportPeople(
  root: IdentityPerson | null,
): Map<string, ReportPerson> {
  const out = new Map<string, ReportPerson>();
  const walk = (node: IdentityPerson): void => {
    if (node.person_id) {
      out.set(normalizePersonId(node.person_id), {
        entityId: normalizePersonId(node.person_id),
        name: personDisplayName(node),
        email: node.email ?? "",
        division: node.division ?? "",
        department: node.department ?? "",
        jobTitle: node.job_title ?? "",
        managerName: node.supervisor_name ?? "",
        managerEmail: node.supervisor_email ?? "",
        status: node.status ?? "",
      });
    }
    node.subordinates.forEach(walk);
  };
  if (root) walk(root);
  return out;
}
