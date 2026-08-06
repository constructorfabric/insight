import type { IdentityPerson } from "@/types/insight";

/**
 * Identity fixtures for portal tests.
 *
 * Person keys are canonical UUIDs since the identity cutover — the route guard
 * (`lib/metrics/entity.ts`) redirects anything else and the metrics API rejects
 * it — which makes hand-written fixtures unreadable. `pid("lead")` mints a
 * stable, valid UUID from a short label so tests stay legible while still
 * exercising the real key shape, and `identityPerson` builds the tree nodes
 * every org view resolves its roster from.
 */

/** A deterministic, valid person UUID for a short fixture label. */
export function pid(label: string): string {
  const hex = [...label]
    .map((c) => c.charCodeAt(0).toString(16).padStart(2, "0"))
    .join("");
  const body = (hex + "1".repeat(32)).slice(0, 32);
  return [
    body.slice(0, 8),
    body.slice(8, 12),
    body.slice(12, 16),
    body.slice(16, 20),
    body.slice(20, 32),
  ].join("-");
}

/**
 * An identity tree node keyed by `pid(label)`, with the label doubling as the
 * display name and the local part of the email (emails still exist on the
 * profile — they are just no longer the key).
 */
export function identityPerson(
  label: string,
  over: Partial<IdentityPerson> = {},
  subordinates: IdentityPerson[] = [],
): IdentityPerson {
  return {
    person_id: pid(label),
    email: `${label}@x`,
    display_name: label,
    subordinates,
    ...over,
  } as unknown as IdentityPerson;
}
