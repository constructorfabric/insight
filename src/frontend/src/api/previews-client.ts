/**
 * Thin client for the previews service (`/api/previews/v1`). Create and
 * delete are gated server-side on the `previews-admin` or `admin` role; the
 * wire is camelCase (unlike identity's snake_case).
 */

import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_PREVIEWS_BASE as string | undefined) ??
  "/api/previews/v1";

/** One experiment as the API serves it. */
export interface Experiment {
  name: string;
  tag: string;
  /** Where the experiment serves: `https://<host>/exp/<name>/`. */
  url: string;
  /** The creating gateway-JWT subject (a person id). */
  creator: string;
  createdAt?: string | null;
  expiresAt?: string | null;
  status: string;
}

export interface ExperimentListResponse {
  experiments: Experiment[];
}

export interface CreateExperimentRequest {
  /** The `/exp/<name>` slug — the server validates (DNS-1123 label, ≤55). */
  name: string;
  /** FE image tag — the server validates (`preview-…` or a CI build tag). */
  tag: string;
}

export class PreviewsApiError extends Error {
  status: number;
  body: unknown;

  constructor(status: number, body: unknown) {
    super(`Previews API ${status}`);
    this.name = "PreviewsApiError";
    this.status = status;
    this.body = body;
  }
}

async function failure(res: Response): Promise<PreviewsApiError> {
  const body = await res.json().catch(() => null);
  return new PreviewsApiError(res.status, body);
}

/** Every live experiment; authenticated-only. */
export async function listExperiments(): Promise<Experiment[]> {
  const res = await fetchWithAuth(`${BASE}/experiments`);
  if (!res.ok) throw await failure(res);
  let listed: ExperimentListResponse;
  try {
    listed = (await res.json()) as ExperimentListResponse;
  } catch {
    throw new PreviewsApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(listed.experiments)) {
    throw new PreviewsApiError(res.status, { error: "malformed_experiments" });
  }
  return listed.experiments;
}

/** Create an experiment (name + tag). Requires previews-admin or admin. */
export async function createExperiment(
  req: CreateExperimentRequest,
): Promise<Experiment> {
  const res = await fetchWithAuth(`${BASE}/experiments`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(req),
  });
  if (!res.ok) throw await failure(res);
  try {
    return (await res.json()) as Experiment;
  } catch {
    throw new PreviewsApiError(res.status, { error: "invalid_json" });
  }
}

/** Delete an experiment by slug. Requires previews-admin or admin. */
export async function deleteExperiment(name: string): Promise<void> {
  const res = await fetchWithAuth(
    `${BASE}/experiments/${encodeURIComponent(name)}`,
    { method: "DELETE" },
  );
  if (!res.ok) throw await failure(res);
}
