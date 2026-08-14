/**
 * The one name a person goes by across every identity-console surface —
 * the card's precedence: display name, else email, else the source-native
 * username (a git-only identity is recognisable by its handle), else the id.
 */
import type { PersonSummary } from "@/api/identity-client";

export function personDisplayName(person: PersonSummary): string {
  return (
    person.display_name?.trim() ||
    person.email?.trim() ||
    person.username?.trim() ||
    person.person_id
  );
}
