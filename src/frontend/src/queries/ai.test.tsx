/**
 * The two gates the tile affordance waits on, and the cache writes that make a
 * saved key take effect without a reload.
 */
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi, beforeEach } from "vitest";

const mocks = vi.hoisted(() => ({
  getAiConfig: vi.fn(),
  getAiCredentialStatus: vi.fn(),
  getAiSettings: vi.fn(),
  listAiContext: vi.fn(),
  putAiCredential: vi.fn(),
  deleteAiCredential: vi.fn(),
  putAiSettings: vi.fn(),
  resetAiSettings: vi.fn(),
  createAiContext: vi.fn(),
  updateAiContext: vi.fn(),
  deleteAiContext: vi.fn(),
  explainMetric: vi.fn(),
}));

vi.mock("@/api/ai-client", () => mocks);

const admin = vi.hoisted(() => ({ isAdmin: true }));
vi.mock("@/queries/identity-me", () => ({
  useIsAdmin: () => admin,
}));

import {
  useAiAvailable,
  useAiContext,
  useAiSettings,
  useCreateAiContext,
  useDeleteAiContext,
  useExplainMetric,
  useForgetAiCredential,
  useResetAiSystemPrompt,
  useSaveAiCredential,
  useSaveAiSystemPrompt,
  useUpdateAiContext,
} from "./ai";

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return {
    client,
    Wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  };
}

beforeEach(() => {
  admin.isAdmin = true;
  Object.values(mocks).forEach((fn) => fn.mockReset());
  mocks.getAiConfig.mockResolvedValue({ enabled: true, model: "m" });
  mocks.getAiCredentialStatus.mockResolvedValue({
    configured: true,
    hint: "wxyz",
  });
  mocks.getAiSettings.mockResolvedValue({
    system_prompt: "BASE",
    is_default: true,
  });
  mocks.listAiContext.mockResolvedValue({
    items: [
      { id: "1", scope: "person", title: "T", body: "B", updated_at: "now" },
    ],
  });
});

describe("useAiAvailable", () => {
  it("opens on a stand that carries its own key, with nothing stored", async () => {
    mocks.getAiConfig.mockResolvedValue({
      enabled: true,
      model: "m",
      stand_key: true,
    });
    mocks.getAiCredentialStatus.mockResolvedValue({
      configured: false,
      hint: "",
    });
    const { Wrapper } = wrapper();

    const { result } = renderHook(() => useAiAvailable(), { wrapper: Wrapper });

    await waitFor(() => expect(result.current.hasKey).toBe(true));
    expect(mocks.getAiCredentialStatus).not.toHaveBeenCalled();
  });

  it("stays shut for a non-admin where the stand allows only admins to ask", async () => {
    admin.isAdmin = false;
    mocks.getAiConfig.mockResolvedValue({
      enabled: true,
      model: "m",
      stand_key: true,
      admin_only: true,
    });
    const { Wrapper } = wrapper();

    const { result } = renderHook(() => useAiAvailable(), { wrapper: Wrapper });

    await waitFor(() => expect(result.current.featureOn).toBe(true));
    expect(result.current.hasKey).toBe(false);
  });

  it("opens for an admin on the same stand", async () => {
    mocks.getAiConfig.mockResolvedValue({
      enabled: true,
      model: "m",
      stand_key: true,
      admin_only: true,
    });
    const { Wrapper } = wrapper();

    const { result } = renderHook(() => useAiAvailable(), { wrapper: Wrapper });

    await waitFor(() => expect(result.current.hasKey).toBe(true));
  });

  it("is open only when the deployment offers it and a key is stored", async () => {
    const { Wrapper } = wrapper();

    const { result } = renderHook(() => useAiAvailable(), { wrapper: Wrapper });

    await waitFor(() =>
      expect(result.current).toEqual({ featureOn: true, hasKey: true })
    );
  });

  it("never asks about a key on a deployment that does not offer explanations", async () => {
    mocks.getAiConfig.mockResolvedValue({ enabled: false, model: "" });
    const { Wrapper } = wrapper();

    const { result } = renderHook(() => useAiAvailable(), { wrapper: Wrapper });

    await waitFor(() => expect(result.current.featureOn).toBe(false));
    expect(result.current.hasKey).toBe(false);
    expect(mocks.getAiCredentialStatus).not.toHaveBeenCalled();
  });
});

describe("the AI settings hooks", () => {
  it("reads the stored prompt and the entry list", async () => {
    const { Wrapper } = wrapper();

    const settings = renderHook(() => useAiSettings(true), {
      wrapper: Wrapper,
    });
    const context = renderHook(() => useAiContext(true), { wrapper: Wrapper });

    await waitFor(() =>
      expect(settings.result.current.data?.is_default).toBe(true)
    );
    await waitFor(() => expect(context.result.current.data).toHaveLength(1));
  });

  it("makes a saved key visible without another read", async () => {
    mocks.putAiCredential.mockResolvedValue({ configured: true, hint: "abcd" });
    const { client, Wrapper } = wrapper();

    const { result } = renderHook(() => useSaveAiCredential(), {
      wrapper: Wrapper,
    });
    result.current.mutate("sk-ant-abcd");

    await waitFor(() =>
      expect(client.getQueryData(["ai", "credentials"])).toEqual({
        configured: true,
        hint: "abcd",
      })
    );
  });

  it("makes a removed key disappear the same way", async () => {
    mocks.deleteAiCredential.mockResolvedValue(undefined);
    const { client, Wrapper } = wrapper();

    const { result } = renderHook(() => useForgetAiCredential(), {
      wrapper: Wrapper,
    });
    result.current.mutate();

    await waitFor(() =>
      expect(client.getQueryData(["ai", "credentials"])).toEqual({
        configured: false,
        hint: "",
      })
    );
  });

  it("stores a written prompt and re-reads after a reset", async () => {
    mocks.putAiSettings.mockResolvedValue({
      system_prompt: "OURS",
      is_default: false,
    });
    mocks.resetAiSettings.mockResolvedValue(undefined);
    const { client, Wrapper } = wrapper();

    const save = renderHook(() => useSaveAiSystemPrompt(), {
      wrapper: Wrapper,
    });
    save.result.current.mutate("OURS");
    await waitFor(() =>
      expect(client.getQueryData(["ai", "settings"])).toEqual({
        system_prompt: "OURS",
        is_default: false,
      })
    );

    const reset = renderHook(() => useResetAiSystemPrompt(), {
      wrapper: Wrapper,
    });
    reset.result.current.mutate();
    await waitFor(() => expect(mocks.resetAiSettings).toHaveBeenCalled());
  });

  it("re-reads the list after every write to it", async () => {
    mocks.createAiContext.mockResolvedValue({
      id: "2",
      scope: "person",
      title: "T2",
      body: "B2",
      updated_at: "now",
    });
    mocks.updateAiContext.mockResolvedValue({
      id: "2",
      scope: "person",
      title: "T3",
      body: "B2",
      updated_at: "now",
    });
    mocks.deleteAiContext.mockResolvedValue(undefined);
    const { Wrapper } = wrapper();

    const create = renderHook(() => useCreateAiContext(), { wrapper: Wrapper });
    create.result.current.mutate({ scope: "person", title: "T2", body: "B2" });
    await waitFor(() => expect(mocks.createAiContext).toHaveBeenCalled());

    const update = renderHook(() => useUpdateAiContext(), { wrapper: Wrapper });
    update.result.current.mutate({ id: "2", body: { title: "T3" } });
    await waitFor(() =>
      expect(mocks.updateAiContext).toHaveBeenCalledWith("2", { title: "T3" })
    );

    const remove = renderHook(() => useDeleteAiContext(), { wrapper: Wrapper });
    remove.result.current.mutate("2");
    await waitFor(() => expect(mocks.deleteAiContext).toHaveBeenCalledWith("2"));
  });

  it("asks for an explanation with the snapshot it was given", async () => {
    mocks.explainMetric.mockResolvedValue({
      text: "…",
      model: "m",
      tenant_context_entries: 0,
      person_context_entries: 1,
    });
    const { Wrapper } = wrapper();
    const snapshot = {
      metric_key: "tasks.closed",
      label: "Tasks closed",
      value: "34",
      period: "month",
      since: "2026-08-01",
      until: "2026-08-22",
      delta: "",
      peer: "",
      help: "",
      trend: [],
    };

    const { result } = renderHook(() => useExplainMetric(), {
      wrapper: Wrapper,
    });
    result.current.mutate(snapshot);

    await waitFor(() =>
      expect(mocks.explainMetric).toHaveBeenCalledWith(snapshot)
    );
  });
});
