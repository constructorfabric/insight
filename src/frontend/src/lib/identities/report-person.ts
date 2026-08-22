import type { PersonSummary } from "@/api/identity-client";
import { personDisplayName } from "@/lib/identities/person-display";
import { normalizePersonId } from "@/lib/metrics/entity";
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

export function reportPeopleInScope(
  root: IdentityPerson | null,
  roster: readonly { person_id: string }[] | null,
): ReportPerson[] | null {
  if (!roster) return null;

  const attributes = collectReportPeople(root);
  return roster.flatMap((entry) => {
    const person = attributes.get(normalizePersonId(entry.person_id));
    return person ? [person] : [];
  });
}

export function reportPeopleInFlatScope(
  roster: readonly PersonSummary[] | null,
): ReportPerson[] | null {
  return roster?.map((person) => ({
    entityId: normalizePersonId(person.person_id),
    name: personDisplayName(person),
    email: person.email ?? "",
    division: "",
    department: "",
    jobTitle: person.job_title ?? "",
    managerName: "",
    managerEmail: "",
    status: person.status ?? "",
  })) ?? null;
}
