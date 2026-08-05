/**
 * Person entity ids are canonical person UUIDs (the identity cutover): the
 * metrics API rejects anything else with a 400, and identity resolves the
 * same id for profiles. Casing is normalized because a UUID may arrive from a
 * route param or an identity record in either case, and query keys plus
 * response lookups compare as strings.
 */
export function normalizePersonId(personId: string): string {
  return personId.trim().toLowerCase();
}

const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const NIL_UUID = "00000000-0000-0000-0000-000000000000";

/**
 * Whether a string is a person UUID the metrics API would accept. Guards the
 * route boundary: an id that fails here must be redirected, not sent to the API
 * where it reads as a 400 the user cannot act on.
 *
 * The nil UUID parses as a UUID but is never a person — both analytics and
 * identity reject it — so it fails here too. Otherwise it would clear the route
 * guard and paint a dashboard whose every metric request 400s.
 */
export function isPersonId(value: string): boolean {
  const normalized = normalizePersonId(value);
  return UUID_RE.test(normalized) && normalized !== NIL_UUID;
}
