/**
 * `subordinates` is empty until the backend `expand_subordinates` flag is on
 * (see `PersonResponse.cs`). When mocks are off and the endpoint fails the
 * caller surfaces the failure to the UI — never silently falls back to seeded
 * data.
 *
 * The legacy `GET /persons/{email}` lookup (RFC 8594 deprecated) is replaced by
 * `POST /profiles` with a `{ value_type, value }` body. The wire shape
 * (`ProfileResponse`) mirrors the C# `PersonResponse`, but nearly every field is
 * optional; we normalize it back into the required-string `IdentityPerson`
 * projection the UI already consumes so callers and the org-tree sidebar are
 * unchanged.
 */

import { fetchWithAuth } from "@/api/fetch-with-auth";
import { normalizePersonId } from "@/lib/metrics/entity";
import type { IdentityPerson } from "@/types/insight";

const BASE =
  (import.meta.env.VITE_IDENTITY_BASE as string | undefined) ??
  "/api/identity/v1";

/** Wire shape of `POST /profiles` (snake_case; optional fields omitted). */
interface ProfileResponse {
  person_id: string;
  insight_tenant_id: string;
  email?: string;
  display_name?: string;
  first_name?: string;
  last_name?: string;
  department?: string;
  division?: string;
  job_title?: string;
  status?: string;
  username?: string;
  employee_id?: string;
  supervisor_email?: string;
  supervisor_name?: string;
  parent_email?: string;
  parent_person_id?: string;
  subordinates?: ProfileResponse[];
  ids?: unknown[];
}

export class IdentityApiError extends Error {
  status: number;
  body: unknown;

  constructor(status: number, body: unknown) {
    super(`Identity API ${status}`);
    this.name = "IdentityApiError";
    this.status = status;
    this.body = body;
  }
}

/**
 * Normalize a `ProfileResponse` into the FE `IdentityPerson`. `person_id` is the
 * UI identity (route links + React keys), so the top-level profile is guaranteed
 * to carry one by `getPerson`; subordinates the wire returns without one are
 * dropped rather than projected to `""` — a keyless node would make broken links
 * and collide as duplicate React keys across siblings. `email` is display-only
 * now and may legitimately be absent. Other optional strings default to `""`;
 * omitted parent/supervisor fields stay `null`.
 */
function toIdentityPerson(p: ProfileResponse): IdentityPerson {
  return {
    person_id: p.person_id,
    email: p.email ?? "",
    display_name: p.display_name ?? "",
    first_name: p.first_name ?? "",
    last_name: p.last_name ?? "",
    department: p.department ?? "",
    division: p.division ?? "",
    job_title: p.job_title ?? "",
    status: p.status ?? "",
    parent_email: p.parent_email ?? null,
    // `parent_id` has no ProfileResponse source; preserve the prior default.
    parent_id: null,
    parent_person_id: p.parent_person_id ?? null,
    supervisor_email: p.supervisor_email ?? null,
    supervisor_name: p.supervisor_name ?? null,
    subordinates: (p.subordinates ?? [])
      .filter((s) => Boolean(s.person_id?.trim()))
      .map(toIdentityPerson),
  };
}

/**
 * Resolve one profile by canonical person id — the key the SPA routes on and
 * the metrics API filters by since the identity cutover. Identity applies the
 * caller's visible set here, so a person's name and their metrics answer to
 * one permission: an id outside it is a 404, not a nameless dashboard.
 */
export async function getPerson(personId: string): Promise<IdentityPerson> {
  // Normalized on the way out, matching the query key: one spelling of an id
  // must not become two requests, or two cache entries for one person.
  return resolveProfile({
    value_type: "person_id",
    value: normalizePersonId(personId),
  });
}

async function resolveProfile(body: {
  value_type: "person_id";
  value: string;
}): Promise<IdentityPerson> {
  const url = `${BASE}/profiles`;
  const res = await fetchWithAuth(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let profile: ProfileResponse;
  try {
    profile = (await res.json()) as ProfileResponse;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  // `person_id` is the queried identity + the UI's key; a profile without it
  // is unusable, so surface it rather than projecting a keyless person. The
  // email is NOT required — identity resolves persons whose observation log
  // carries no current email, and nothing keys on it any more.
  if (!profile.person_id?.trim()) {
    throw new IdentityApiError(res.status, { error: "missing_person_id" });
  }
  return toIdentityPerson(profile);
}
