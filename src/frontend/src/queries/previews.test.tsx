/** The previews gate fails closed, and mutations invalidate the listing. */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as previewsClient from "@/api/previews-client";
import { makeSession } from "@/test/session";
import type { Session } from "@/auth/types";

vi.mock("@/api/previews-client");

const auth = vi.hoisted(() => ({ session: null as unknown }));
vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: auth.session }),
}));

import {
  canManagePreviews,
  useCreateExperiment,
  useDeleteExperiment,
  useExperiments,
  usePreviewsGate,
} from "./previews";

const listExperiments = vi.mocked(previewsClient.listExperiments);
const createExperiment = vi.mocked(previewsClient.createExperiment);
const deleteExperiment = vi.mocked(previewsClient.deleteExperiment);

function harness() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  return { wrapper, queryClient };
}

const EXPERIMENT: previewsClient.Experiment = {
  name: "my-experiment",
  tag: "preview-my-branch",
  url: "https://preview.example.com/exp/my-experiment/",
  creator: "00000000-0000-0000-0000-000000000001",
  status: "ready",
};

function previewsSession(overrides: Partial<Session> = {}): Session {
  return makeSession({
    experimentsEnabled: true,
    roles: ["previews-admin"],
    ...overrides,
  });
}

beforeEach(() => {
  vi.resetAllMocks();
  auth.session = previewsSession();
});

describe("canManagePreviews", () => {
  it("opens only when the capability AND a managing role are both present", () => {
    for (const [caseName, session, allowed] of [
      ["previews-admin on an enabled stand", previewsSession(), true],
      [
        "admin on an enabled stand",
        previewsSession({ roles: ["user", "admin"] }),
        true,
      ],
      [
        "managing role but the capability is off",
        previewsSession({ experimentsEnabled: false }),
        false,
      ],
      [
        "capability on but only the default role",
        previewsSession({ roles: ["user"] }),
        false,
      ],
      ["no roles at all", previewsSession({ roles: [] }), false],
      [
        "a role that merely contains the name",
        previewsSession({ roles: ["previews-admin-plus"] }),
        false,
      ],
      ["no session", null, false],
    ] as const) {
      expect(canManagePreviews(session), `case: ${caseName}`).toBe(allowed);
    }
  });
});

describe("usePreviewsGate", () => {
  it("reads the live session", () => {
    const { result } = renderHook(() => usePreviewsGate(), harness());
    expect(result.current).toBe(true);

    auth.session = null;
    const { result: closed } = renderHook(() => usePreviewsGate(), harness());
    expect(closed.current).toBe(false);
  });
});

describe("useExperiments", () => {
  it("serves the listing", async () => {
    listExperiments.mockResolvedValueOnce([EXPERIMENT]);

    const { result } = renderHook(() => useExperiments(), harness());

    await waitFor(() => expect(result.current.isPending).toBe(false));
    expect(result.current.data).toEqual([EXPERIMENT]);
  });

  it("never asks without a session", async () => {
    auth.session = null;

    renderHook(() => useExperiments(), harness());

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(listExperiments).not.toHaveBeenCalled();
  });
});

describe("mutations", () => {
  it("create invalidates the listing so the console reconciles", async () => {
    listExperiments.mockResolvedValue([EXPERIMENT]);
    createExperiment.mockResolvedValueOnce(EXPERIMENT);
    const h = harness();
    const invalidate = vi.spyOn(h.queryClient, "invalidateQueries");

    const { result } = renderHook(() => useCreateExperiment(), h);
    result.current.mutate({ name: "my-experiment", tag: "preview-my-branch" });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["previews"] });
  });

  it("delete invalidates the listing too", async () => {
    deleteExperiment.mockResolvedValueOnce(undefined);
    const h = harness();
    const invalidate = vi.spyOn(h.queryClient, "invalidateQueries");

    const { result } = renderHook(() => useDeleteExperiment(), h);
    result.current.mutate("my-experiment");

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["previews"] });
  });

  it("a refused create surfaces the error instead of pretending success", async () => {
    createExperiment.mockRejectedValueOnce(new Error("Previews API 403"));

    const { result } = renderHook(() => useCreateExperiment(), harness());
    result.current.mutate({ name: "x", tag: "preview-x" });

    await waitFor(() => expect(result.current.isError).toBe(true));
  });
});
