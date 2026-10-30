import {
  EVERYONE,
  type DashboardVisibility,
  type Grantee,
} from "@/api/custom-client";
import type { PersonSummary } from "@/api/identity-client";

export function granteesFor(
  visibility: DashboardVisibility,
  people: readonly PersonSummary[]
): Grantee[] {
  switch (visibility) {
    case "private":
      return [];
    case "people":
      return people.map((person) => person.person_id);
    case "everyone":
      return [EVERYONE];
  }
}

export function audienceChosen(
  visibility: DashboardVisibility,
  people: readonly PersonSummary[]
): boolean {
  return visibility !== "people" || people.length > 0;
}
