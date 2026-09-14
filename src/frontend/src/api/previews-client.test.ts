import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  createExperiment,
  deleteExperiment,
  listExperiments,
  listImages,
  PreviewsApiError,
} from "./previews-client";

const mockFetch = fetchWithAuth as unknown as ReturnType<typeof vi.fn>;

function response(
  body: unknown,
  init?: { ok?: boolean; status?: number }
): Response {
  return {
    ok: init?.ok ?? true,
    status: init?.status ?? 200,
    json: async () => body,
  } as unknown as Response;
}

const EXPERIMENT = {
  name: "my-experiment",
  tag: "preview-my-branch",
  url: "https://preview.example.com/exp/my-experiment/",
  creator: "00000000-0000-0000-0000-000000000001",
  status: "ready",
} as const;

beforeEach(() => {
  mockFetch.mockReset();
});

describe("listExperiments", () => {
  it("serves the list with its count and cap", async () => {
    const listed = { experiments: [EXPERIMENT], liveCount: 1, cap: 10 };
    mockFetch.mockResolvedValueOnce(response(listed));

    await expect(listExperiments()).resolves.toEqual(listed);
    expect(mockFetch).toHaveBeenCalledWith("/api/previews/v1/experiments");
  });

  it("raises a malformed answer rather than resolving to an empty list", async () => {
    mockFetch.mockResolvedValueOnce(response({ nope: true }));

    await expect(listExperiments()).rejects.toBeInstanceOf(PreviewsApiError);
  });

  it("raises the refusal with its status", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "unauthenticated" }, { ok: false, status: 401 })
    );

    await expect(listExperiments()).rejects.toMatchObject({ status: 401 });
  });
});

describe("listImages", () => {
  it("serves the tags with the configured flag", async () => {
    const listed = { configured: true, tags: ["preview-my-branch"] };
    mockFetch.mockResolvedValueOnce(response(listed));

    await expect(listImages()).resolves.toEqual(listed);
    expect(mockFetch).toHaveBeenCalledWith("/api/previews/v1/images");
  });

  it("raises a malformed answer rather than inventing tags", async () => {
    mockFetch.mockResolvedValueOnce(response({ configured: true }));

    await expect(listImages()).rejects.toBeInstanceOf(PreviewsApiError);
  });

  it("raises the refusal with its status", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "unavailable" }, { ok: false, status: 503 })
    );

    await expect(listImages()).rejects.toMatchObject({ status: 503 });
  });
});

describe("createExperiment", () => {
  it("posts name and tag as camelCase JSON", async () => {
    mockFetch.mockResolvedValueOnce(response(EXPERIMENT, { status: 201 }));

    await createExperiment({ name: "my-experiment", tag: "preview-my-branch" });

    expect(mockFetch).toHaveBeenCalledWith("/api/previews/v1/experiments", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "my-experiment", tag: "preview-my-branch" }),
    });
  });

  it("raises the server's refusal — the 403 is the boundary, not the UI", async () => {
    mockFetch.mockResolvedValueOnce(
      response(
        {
          detail:
            "managing experiments requires the previews-admin or admin role",
        },
        { ok: false, status: 403 }
      )
    );

    await expect(
      createExperiment({ name: "x", tag: "preview-x" })
    ).rejects.toMatchObject({ status: 403 });
  });
});

describe("deleteExperiment", () => {
  it("addresses the experiment by its escaped slug", async () => {
    mockFetch.mockResolvedValueOnce(response(null, { status: 204 }));

    await deleteExperiment("my-experiment");

    expect(mockFetch).toHaveBeenCalledWith(
      "/api/previews/v1/experiments/my-experiment",
      { method: "DELETE" }
    );
  });

  it("raises a refusal rather than reporting a delete that never happened", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ detail: "experiment not found" }, { ok: false, status: 404 })
    );

    await expect(deleteExperiment("gone")).rejects.toBeInstanceOf(
      PreviewsApiError
    );
  });
});
