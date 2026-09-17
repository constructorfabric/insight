vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
// The real error class, so `refusal` can read the body off it: an automocked
// constructor never runs, and every refusal would read as the fallback.
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchDataset: vi.fn(),
    fetchDatasetRecords: vi.fn(),
    fetchDatasetDependents: vi.fn(),
    deleteDataset: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.datasets.$name";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

const COMMITS = {
  name: "commits",
  declaration: {
    title: "Commits",
    description: "One per commit.",
    fields: [
      {
        name: "author",
        path: "who.email",
        type: "string" as const,
        person: "email" as const,
      },
      {
        name: "day",
        path: "day",
        type: "datetime" as const,
        default_clock: true,
      },
    ],
    row_identity: ["author"],
  },
};

function onDataset(name = "commits") {
  portalRouter.go(`/portal/custom/datasets/${name}`);
}

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset();
  onDataset();
  vi.mocked(customClient.fetchDatasetRecords).mockResolvedValue([]);
  vi.mocked(customClient.fetchDatasetDependents).mockResolvedValue([]);
});

describe("/portal/custom/datasets/$name", () => {
  it("reads the declaration back, field by field", async () => {
    vi.mocked(customClient.fetchDataset).mockResolvedValue(COMMITS);

    render(<Component />, { wrapper });

    expect(await screen.findByText("One per commit.")).toBeInTheDocument();
    expect(await screen.findByText("author")).toBeInTheDocument();
    expect(await screen.findByText("main date")).toBeInTheDocument();
    expect(await screen.findByText("person")).toBeInTheDocument();
    expect(
      await screen.findByText("One record per author.")
    ).toBeInTheDocument();
  });

  it("shows the latest records as they arrived", async () => {
    vi.mocked(customClient.fetchDataset).mockResolvedValue(COMMITS);
    vi.mocked(customClient.fetchDatasetRecords).mockResolvedValue([
      {
        id: "1",
        received_at: "2026-09-17 10:00:00",
        raw_data: { lines: 7 },
      },
    ]);

    render(<Component />, { wrapper });

    expect(await screen.findByText('{"lines":7}')).toBeInTheDocument();
  });

  it("names the metrics that read it", async () => {
    vi.mocked(customClient.fetchDataset).mockResolvedValue(COMMITS);
    vi.mocked(customClient.fetchDatasetDependents).mockResolvedValue([
      "lines_per_day",
    ]);

    render(<Component />, { wrapper });

    expect(await screen.findByText("lines_per_day")).toBeInTheDocument();
  });

  // A dataset mid-create or mid-removal is not found, and the page says which
  // one rather than asking for records that are not there.
  it("says which dataset is missing rather than reading anything else", async () => {
    vi.mocked(customClient.fetchDataset).mockRejectedValue(
      new customClient.CustomApiError(404, {
        detail: "dataset `commits` was not found",
      })
    );

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "dataset `commits` was not found"
    );
    expect(customClient.fetchDatasetRecords).not.toHaveBeenCalled();
    expect(customClient.fetchDatasetDependents).not.toHaveBeenCalled();
  });

  it("asks before taking a dataset away, and says what still reads it", async () => {
    vi.mocked(customClient.fetchDataset).mockResolvedValue(COMMITS);
    vi.mocked(customClient.deleteDataset).mockRejectedValue(
      new customClient.CustomApiError(409, {
        context: {
          violations: [{ description: "still read by lines_per_day" }],
        },
      })
    );

    render(<Component />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Remove" })
    );
    await userEvent.click(
      await screen.findByRole("button", { name: /Remove it, with its records/ })
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "still read by lines_per_day"
    );
  });

  it("goes back to the catalogue once the dataset is gone", async () => {
    vi.mocked(customClient.fetchDataset).mockResolvedValue(COMMITS);
    vi.mocked(customClient.deleteDataset).mockResolvedValue(undefined);

    render(<Component />, { wrapper });
    await userEvent.click(
      await screen.findByRole("button", { name: "Remove" })
    );
    await userEvent.click(
      await screen.findByRole("button", { name: /Remove it, with its records/ })
    );

    expect(portalRouter.pathname).toBe("/portal/custom/datasets");
  });
});
