import type {
  Alert,
  AlertDestination,
  AlertDraft,
  AlertPage,
  NotificationPage,
} from "@/api/alerts-types";
import {
  BASE,
  CustomApiError,
  JSON_HEADERS,
  ensureOk,
  readJson,
} from "@/api/custom-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

export type * from "@/api/alerts-types";

/**
 * An alert's id as a path segment.
 *
 * INVARIANT: an empty id is never a request. It would address the collection
 * with a trailing slash, which no route serves, and the unrouted 401 reads as a
 * session that ended.
 */
function alertPath(id: string): string {
  if (id === "") {
    throw new CustomApiError(400, { detail: "An alert id is required." });
  }
  return `${BASE}/alerts/${encodeURIComponent(id)}`;
}

function sliceQuery(slice: {
  search?: string;
  limit?: number;
  offset?: number;
}): string {
  const query = new URLSearchParams();
  if (slice.search) query.set("q", slice.search);
  if (slice.limit !== undefined) query.set("limit", String(slice.limit));
  if (slice.offset) query.set("offset", String(slice.offset));
  const asked = query.toString();

  return asked ? `?${asked}` : "";
}

export async function fetchAlerts(
  slice: { search?: string; limit?: number; offset?: number } = {}
): Promise<AlertPage> {
  const res = await fetchWithAuth(`${BASE}/alerts${sliceQuery(slice)}`);
  return readJson<AlertPage>(res);
}

export async function fetchAlert(id: string): Promise<Alert> {
  const res = await fetchWithAuth(alertPath(id));
  return readJson<Alert>(res);
}

export async function createAlert(draft: AlertDraft): Promise<Alert> {
  const res = await fetchWithAuth(`${BASE}/alerts`, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(draft),
  });
  return readJson<Alert>(res);
}

/**
 * Replaces an alert at the revision the draft expects.
 *
 * Refused with 409 when the alert has moved on since it was read, so a
 * change made elsewhere is never silently overwritten.
 */
export async function replaceAlert(
  id: string,
  draft: AlertDraft & { expected_revision: number }
): Promise<Alert> {
  const res = await fetchWithAuth(alertPath(id), {
    method: "PUT",
    headers: JSON_HEADERS,
    body: JSON.stringify(draft),
  });
  return readJson<Alert>(res);
}

export async function setAlertEnabled(
  id: string,
  enabled: boolean,
  expectedRevision: number
): Promise<Alert> {
  const res = await fetchWithAuth(
    `${alertPath(id)}/${enabled ? "enable" : "disable"}`,
    {
      method: "POST",
      headers: JSON_HEADERS,
      body: JSON.stringify({ expected_revision: expectedRevision }),
    }
  );
  return readJson<Alert>(res);
}

/** Removes an alert, with its schedule and every notification it owed. */
export async function deleteAlert(id: string): Promise<void> {
  const res = await fetchWithAuth(alertPath(id), { method: "DELETE" });
  await ensureOk(res);
}

export async function fetchAlertNotifications(
  id: string,
  slice: { limit?: number; offset?: number } = {}
): Promise<NotificationPage> {
  const res = await fetchWithAuth(
    `${alertPath(id)}/notifications${sliceQuery(slice)}`
  );
  return readJson<NotificationPage>(res);
}

export async function fetchAlertDestinations(): Promise<AlertDestination[]> {
  const res = await fetchWithAuth(`${BASE}/alert-destinations`);
  const read = await readJson<{ destinations: AlertDestination[] }>(res);

  return read.destinations;
}
