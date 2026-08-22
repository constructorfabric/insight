/**
 * The one name a person goes by across every surface that labels a person.
 *
 * Precedence: display name, else the source-native username, else the e-mail.
 * The address comes LAST on purpose — a source that hides a member's e-mail
 * still reports one for their commits, and an address generated from the
 * account (`<id>+<handle>@…`) is not what anybody calls that person. The handle
 * is.
 */

/** The naming fields the precedence reads — any person shape can supply them. */
export interface PersonNaming {
  display_name?: string | null;
  username?: string | null;
  email?: string | null;
}

/**
 * The name the journal knows, or null when it knows none.
 *
 * INVARIANT: the one place this precedence is written down. It matches the
 * roster listing's own order key (`person_listing`'s `LABEL_CTES`) — a row
 * shown under one name and sorted under another pages unpredictably.
 */
export function personName(person: PersonNaming): string | null {
  return (
    person.display_name?.trim() ||
    person.username?.trim() ||
    person.email?.trim() ||
    null
  );
}

/** The same name, falling back to the id — always something to render. */
export function personDisplayName(
  person: PersonNaming & { person_id: string },
): string {
  return personName(person) ?? person.person_id;
}
