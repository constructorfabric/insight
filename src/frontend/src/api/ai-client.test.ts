/**
 * The wire contract of the AI assistant client.
 *
 * The key deserves its own assertions: it may travel out in a request body and
 * must never come back in a response the client would then hold.
 */
import { describe, expect, it, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => ({
  fetchWithAuth: vi.fn(),
}));

vi.mock("@/api/fetch-with-auth", () => ({
  fetchWithAuth: mocks.fetchWithAuth,
}));

import {
  createAiContext,
  deleteAiContext,
  deleteAiCredential,
  explainMetric,
  getAiConfig,
  getAiCredentialStatus,
  getAiSettings,
  listAiContext,
  putAiCredential,
  putAiSettings,
  resetAiSettings,
  updateAiContext,
  type MetricSnapshot,
} from "./ai-client";
import { AnalyticsApiError } from "@/api/analytics-client";

const BASE = "/api/analytics/v1";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function noContent(): Response {
  return new Response(null, { status: 204 });
}

function lastCall(): [string, RequestInit] {
  const call = mocks.fetchWithAuth.mock.calls.at(-1);
  return [String(call?.[0]), (call?.[1] ?? {}) as RequestInit];
}

const SNAPSHOT: MetricSnapshot = {
  metric_key: "tasks.closed",
  label: "Tasks closed",
  value: "34",
  period: "month",
  since: "2026-08-01",
  until: "2026-08-22",
  delta: "+6 since last month",
  peer: "Team median 27",
  help: "",
  trend: [1, null, 3],
};

beforeEach(() => {
  mocks.fetchWithAuth.mockReset();
});

describe("ai-client", () => {
  it("reads whether the deployment offers explanations", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({ enabled: true, model: "claude-sonnet-5" })
    );

    await expect(getAiConfig()).resolves.toEqual({
      enabled: true,
      model: "claude-sonnet-5",
    });
    expect(lastCall()[0]).toBe(`${BASE}/ai/config`);
  });

  it("reads the stored-key state without ever receiving a key", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({ configured: true, hint: "wxyz" })
    );

    const status = await getAiCredentialStatus();

    expect(status).toEqual({ configured: true, hint: "wxyz" });
    expect(Object.keys(status)).not.toContain("token");
  });

  it("sends the key in the body and nowhere else", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({ configured: true, hint: "wxyz" })
    );

    await putAiCredential("sk-ant-secret-wxyz");

    const [url, init] = lastCall();
    expect(url).toBe(`${BASE}/ai/credentials`);
    expect(init.method).toBe("PUT");
    expect(init.body).toBe(JSON.stringify({ token: "sk-ant-secret-wxyz" }));
    expect(url).not.toContain("sk-ant");
  });

  it("forgets the key", async () => {
    mocks.fetchWithAuth.mockImplementation(() => noContent());

    await expect(deleteAiCredential()).resolves.toBeUndefined();
    expect(lastCall()[1].method).toBe("DELETE");
  });

  it("reads the system prompt and whether it is the shipped one", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({ system_prompt: "BASE", is_default: true })
    );

    await expect(getAiSettings()).resolves.toEqual({
      system_prompt: "BASE",
      is_default: true,
    });
  });

  it("writes and resets the system prompt", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({ system_prompt: "OURS", is_default: false })
    );
    await putAiSettings("OURS");
    expect(lastCall()[1].body).toBe(
      JSON.stringify({ system_prompt: "OURS" })
    );

    mocks.fetchWithAuth.mockImplementation(() => noContent());
    await resetAiSettings();
    expect(lastCall()[1].method).toBe("DELETE");
  });

  it("lists, adds, edits and removes context entries", async () => {
    mocks.fetchWithAuth.mockImplementation(() => jsonResponse({ items: [] }));
    await expect(listAiContext()).resolves.toEqual({ items: [] });

    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({
        id: "1",
        scope: "person",
        title: "T",
        body: "B",
        updated_at: "now",
      })
    );
    await createAiContext({ scope: "person", title: "T", body: "B" });
    expect(lastCall()[1].method).toBe("POST");

    await updateAiContext("1", { title: "T2" });
    const [patchUrl, patchInit] = lastCall();
    expect(patchUrl).toBe(`${BASE}/ai/context/1`);
    expect(patchInit.method).toBe("PATCH");

    mocks.fetchWithAuth.mockImplementation(() => noContent());
    await deleteAiContext("1");
    expect(lastCall()[0]).toBe(`${BASE}/ai/context/1`);
  });

  it("asks for an explanation with the tile as the body", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({
        text: "…",
        model: "claude-sonnet-5",
        tenant_context_entries: 1,
        person_context_entries: 2,
      })
    );

    const answer = await explainMetric(SNAPSHOT);

    expect(answer.model).toBe("claude-sonnet-5");
    expect(lastCall()[1].body).toBe(JSON.stringify(SNAPSHOT));
  });

  it("raises the API error a refused call carries", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({ detail: "no key" }, 400)
    );

    await expect(explainMetric(SNAPSHOT)).rejects.toBeInstanceOf(
      AnalyticsApiError
    );
  });

  it("raises rather than returning a half-read body", async () => {
    mocks.fetchWithAuth.mockImplementation(
      () => new Response("not json", { status: 200 })
    );

    await expect(getAiConfig()).rejects.toBeInstanceOf(AnalyticsApiError);
  });

  it("raises when a no-content call is refused", async () => {
    mocks.fetchWithAuth.mockImplementation(() =>
      jsonResponse({ detail: "nope" }, 403)
    );

    await expect(deleteAiCredential()).rejects.toBeInstanceOf(
      AnalyticsApiError
    );
  });
});
