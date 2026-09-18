vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    createFileRoute: () => (options: Record<string, unknown>) => options,
  };
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    fetchMetric: vi.fn(),
    fetchWidget: vi.fn(),
    fetchDashboard: vi.fn(),
    fetchDataset: vi.fn(),
    deleteDataset: vi.fn(),
    renameDefinition: vi.fn(),
    putDefinition: vi.fn(),
    fetchMetricNames: vi.fn(),
    fetchWidgetNames: vi.fn(),
    fetchDashboardNames: vi.fn(),
    fetchDatasetNames: vi.fn(),
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { portalRouter } from "@/test/portal-router";

import { Route } from "./portal.custom.edit.$kind.$name";

const Component = (Route as unknown as { component: () => React.ReactNode })
  .component;

function freshClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}

function wrapper({ children }: { children: ReactNode }) {
  return createElement(
    QueryClientProvider,
    { client: freshClient() },
    children
  );
}

/**
 * One cache across several renders, as one session of the portal has - with
 * the portal's own hour of freshness, so a reopened editor meets what a reader
 * meets rather than a cache that always refetches.
 */
function sharing(client: QueryClient) {
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

const NO_NAMES = { names: [], total: 0 };

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/edit/metrics/lines_per_day");
  for (const names of [
    customClient.fetchMetricNames,
    customClient.fetchWidgetNames,
    customClient.fetchDashboardNames,
    customClient.fetchDatasetNames,
  ]) {
    vi.mocked(names).mockResolvedValue(NO_NAMES);
  }
  vi.mocked(customClient.putDefinition).mockResolvedValue(undefined);
  vi.mocked(customClient.fetchDataset).mockImplementation(async (name) => ({
    name,
    declaration: { title: name, fields: [] },
  }));
});

describe("/portal/custom/edit/$kind/$name", () => {
  it("opens the stored definition in the fields its kind admits", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      definition: {
        dataset: "commits",
        fields: [{ field: "lines", type: "int", agg: "sum", as_name: "total" }],
        limit: 25,
      },
    });

    render(<Component />, { wrapper });

    expect(await screen.findByLabelText("Dataset")).toHaveValue("commits");
    expect(screen.getByLabelText("Limit")).toHaveValue(25);
    expect(customClient.fetchMetric).toHaveBeenCalledWith("lines_per_day");
  });

  it("returns to the catalogue once it is stored", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      definition: { dataset: "commits", fields: [] },
    });

    render(<Component />, { wrapper });
    await screen.findByLabelText("Dataset");

    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: "/portal/custom/metrics",
      })
    );
  });

  // A definition may be gone by the time its editor is opened, and an empty
  // form would store a new one over the reader's back.
  it("says so rather than offering an empty form when it is not there", async () => {
    vi.mocked(customClient.fetchMetric).mockRejectedValue(
      new CustomApiError(404, { detail: "no such metric" })
    );

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "no such metric"
    );
    expect(screen.queryByLabelText("Dataset")).not.toBeInTheDocument();
  });

  it("says so for a path naming a kind there is no editor for", () => {
    portalRouter.reset("/portal/custom/edit/sprockets/anything");

    render(<Component />, { wrapper });

    expect(
      screen.getByText("There is no such kind of definition.")
    ).toBeInTheDocument();
  });

  // The editor seeds itself once from what it is handed. A body kept from the
  // last time this definition was opened would be edited and saved over
  // whatever changed it since.
  it("opens what the definition is now, not what it was last time", async () => {
    const session = sharing(
      new QueryClient({
        defaultOptions: { queries: { retry: false, staleTime: 60 * 60_000 } },
      })
    );
    vi.mocked(customClient.fetchMetric).mockResolvedValueOnce({
      definition: { dataset: "commits", fields: [] },
    });

    const first = render(<Component />, { wrapper: session });
    expect(await screen.findByLabelText("Dataset")).toHaveValue("commits");
    first.unmount();

    vi.mocked(customClient.fetchMetric).mockResolvedValueOnce({
      definition: { dataset: "pull_requests", fields: [] },
    });

    render(<Component />, { wrapper: session });

    expect(await screen.findByLabelText("Dataset")).toHaveValue(
      "pull_requests"
    );
  });

  it("asks before taking a dataset away, and says what still reads it", async () => {
    const user = userEvent.setup();
    portalRouter.reset("/portal/custom/edit/datasets/commits");
    vi.mocked(customClient.fetchDataset).mockResolvedValue({
      name: "commits",
      declaration: { title: "Commits", fields: [] },
    });
    vi.mocked(customClient.deleteDataset).mockRejectedValue(
      new CustomApiError(409, {
        context: {
          violations: [
            {
              type: "in_use",
              subject: "commits",
              description: "still read by lines_per_day",
            },
          ],
        },
      })
    );

    render(<Component />, { wrapper });
    await user.click(
      await screen.findByRole("button", { name: "Remove commits" })
    );

    expect(customClient.deleteDataset).not.toHaveBeenCalled();

    await user.click(
      screen.getByRole("button", { name: /Remove it, with its records/ })
    );

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "still read by lines_per_day"
    );
    // The choice stays open: the reader may fix what reads it and try again.
    expect(
      screen.getByRole("button", { name: /Remove it, with its records/ })
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Keep" })).toBeInTheDocument();
  });

  it("follows a metric to its new name once renamed", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetric).mockResolvedValue({
      definition: { dataset: "commits", fields: [] },
    });
    vi.mocked(customClient.renameDefinition).mockResolvedValue({
      name: "lines_daily",
      rewritten: [],
    });

    render(<Component />, { wrapper });
    await screen.findByLabelText("Dataset");
    await user.click(
      screen.getByRole("button", { name: "Rename lines_per_day" })
    );
    const field = screen.getByRole("textbox", {
      name: "New name for lines_per_day",
    });
    await user.clear(field);
    await user.type(field, "lines_daily");
    await user.click(screen.getByRole("button", { name: "Rename" }));

    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: "/portal/custom/edit/$kind/$name",
        params: { kind: "metrics", name: "lines_daily" },
      })
    );
  });

  it("goes back to the catalogue once the dataset is gone", async () => {
    const user = userEvent.setup();
    portalRouter.reset("/portal/custom/edit/datasets/commits");
    vi.mocked(customClient.fetchDataset).mockResolvedValue({
      name: "commits",
      declaration: { title: "Commits", fields: [] },
    });
    vi.mocked(customClient.deleteDataset).mockResolvedValue(undefined);

    render(<Component />, { wrapper });
    await user.click(
      await screen.findByRole("button", { name: "Remove commits" })
    );
    await user.click(
      screen.getByRole("button", { name: /Remove it, with its records/ })
    );

    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: "/portal/custom/datasets",
      })
    );
  });

  it.each([
    {
      kind: "datasets",
      arrange: () =>
        vi.mocked(customClient.fetchDataset).mockResolvedValue({
          name: "commits",
          declaration: { title: "Commits", fields: [] },
        }),
      label: "Title",
      value: "Commits",
    },
    {
      kind: "widgets",
      arrange: () =>
        vi.mocked(customClient.fetchWidget).mockResolvedValue({
          type: "stat",
          metric: "commits",
          value: "total",
        }),
      label: "Metric",
      value: "commits",
    },
    {
      kind: "dashboards",
      arrange: () =>
        vi.mocked(customClient.fetchDashboard).mockResolvedValue({
          title: "Board",
          items: [],
        }),
      label: "Title",
      value: "Board",
    },
  ])(
    "opens a stored $kind body as the document",
    async ({ kind, arrange, label, value }) => {
      portalRouter.reset(`/portal/custom/edit/${kind}/thing`);
      arrange();

      render(<Component />, { wrapper });

      expect(await screen.findByLabelText(label)).toHaveValue(value);
    }
  );
});
