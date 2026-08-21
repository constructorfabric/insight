/**
 * The one name a person goes by across every identity-console surface.
 *
 * Precedence: display name, else the source-native username, else the e-mail.
 * The address comes LAST on purpose — a source that hides a member's e-mail
 * still reports one for their commits, and an address generated from the
 * account (`<id>+<handle>@…`) is not what anybody calls that person. The handle
 * is.
 */
import type { PersonSummary } from "@/api/identity-client";

/**
 * The name the journal knows, or null when it knows none.
 *
 * INVARIANT: the one place this precedence is written down. It matches the
 * roster listing's own order key (`person_listing`'s `LABEL_CTES`) — a row
 * shown under one name and sorted under another pages unpredictably.
 */
export function personName(person: PersonSummary): string | null {
  return (
    person.display_name?.trim() ||
    person.username?.trim() ||
    person.email?.trim() ||
    null
  );
}

/** The same name, falling back to the id — always something to render. */
export function personDisplayName(person: PersonSummary): string {
  return personName(person) ?? person.person_id;
}
