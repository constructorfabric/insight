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
    fetchDefinition: vi.fn(),
    fetchMetric: vi.fn(),
    fetchDataset: vi.fn(),
    deleteDataset: vi.fn(),
    putDefinition: vi.fn(),
    fetchMetrics: vi.fn(),
    fetchWidgets: vi.fn(),
    fetchDashboards: vi.fn(),
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
import { datasetNames } from "@/test/catalogue";
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

const NO_ITEMS = { items: [], total: 0 };

/** The document the editor holds, once it has opened. */
async function shownDocument(): Promise<unknown> {
  const text = await screen.findByLabelText(/as text/i);
  return JSON.parse((text as HTMLTextAreaElement).value);
}

function storedMetric(body: Record<string, unknown>) {
  return { id: "lines_per_day", name: "Lines per day", body };
}

beforeEach(() => {
  vi.resetAllMocks();
  portalRouter.reset("/portal/custom/edit/metrics/lines_per_day");
  for (const page of [
    customClient.fetchMetrics,
    customClient.fetchWidgets,
    customClient.fetchDashboards,
  ]) {
    vi.mocked(page).mockResolvedValue(NO_ITEMS);
  }
  vi.mocked(customClient.fetchDatasetNames).mockResolvedValue(datasetNames([]));
  vi.mocked(customClient.putDefinition).mockResolvedValue(undefined);
  vi.mocked(customClient.fetchDataset).mockImplementation(async (name) => ({
    name,
    declaration: { title: name, fields: [] },
  }));
});

describe("/portal/custom/edit/$kind/$name", () => {
  it("opens the stored definition as its text", async () => {
    const body = {
      dataset: "commits",
      fields: [{ field: "lines", type: "int", agg: "sum", as_name: "total" }],
      limit: 25,
    };
    vi.mocked(customClient.fetchDefinition).mockResolvedValue(
      storedMetric(body)
    );

    render(<Component />, { wrapper });

    expect(await shownDocument()).toEqual(body);
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "Lines per day"
    );
    expect(customClient.fetchDefinition).toHaveBeenCalledWith(
      "metrics",
      "lines_per_day"
    );
  });

  it("returns to the catalogue once it is stored", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchDefinition).mockResolvedValue(
      storedMetric({ dataset: "commits", fields: [] })
    );

    render(<Component />, { wrapper });
    await screen.findByLabelText(/as text/i);

    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "lines_per_day",
        "Lines per day",
        { dataset: "commits", fields: [] }
      )
    );
    await waitFor(() =>
      expect(portalRouter.navigations).toContainEqual({
        to: "/portal/custom/metrics",
      })
    );
  });

  // A definition may be gone by the time its editor is opened, and an empty
  // form would store a new one over the reader's back.
  it("says so rather than offering an empty form when it is not there", async () => {
    vi.mocked(customClient.fetchDefinition).mockRejectedValue(
      new CustomApiError(404, { detail: "no such metric" })
    );

    render(<Component />, { wrapper });

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "no such metric"
    );
    expect(screen.queryByLabelText(/as text/i)).not.toBeInTheDocument();
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
    vi.mocked(customClient.fetchDefinition).mockResolvedValueOnce(
      storedMetric({ dataset: "commits", fields: [] })
    );

    const first = render(<Component />, { wrapper: session });
    expect(await shownDocument()).toMatchObject({ dataset: "commits" });
    first.unmount();

    vi.mocked(customClient.fetchDefinition).mockResolvedValueOnce(
      storedMetric({ dataset: "pull_requests", fields: [] })
    );

    render(<Component />, { wrapper: session });

    expect(await shownDocument()).toMatchObject({
      dataset: "pull_requests",
    });
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

  it("renames a metric by saving it under the new name, keeping its id", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchDefinition).mockResolvedValue(
      storedMetric({ dataset: "commits", fields: [] })
    );

    render(<Component />, { wrapper });
    await screen.findByLabelText(/as text/i);
    expect(
      screen.queryByRole("button", { name: /^Rename/ })
    ).not.toBeInTheDocument();

    const field = screen.getByRole("textbox", { name: "Name" });
    await user.clear(field);
    await user.type(field, "Lines daily");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "lines_per_day",
        "Lines daily",
        { dataset: "commits", fields: [] }
      )
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
      shown: { title: "Commits" },
    },
    {
      kind: "widgets",
      arrange: () => {
        vi.mocked(customClient.fetchDefinition).mockResolvedValue({
          id: "thing",
          name: "Thing",
          body: { type: "stat", metric: "commits", value: "total" },
        });
        vi.mocked(customClient.fetchMetric).mockRejectedValue(
          new CustomApiError(404, null)
        );
      },
      shown: { type: "stat", metric: "commits", value: "total" },
    },
    {
      kind: "dashboards",
      arrange: () =>
        vi.mocked(customClient.fetchDefinition).mockResolvedValue({
          id: "thing",
          name: "Thing",
          body: { title: "Board", items: [] },
        }),
      shown: { title: "Board", items: [] },
    },
  ])(
    "opens a stored $kind body as the document",
    async ({ kind, arrange, shown }) => {
      portalRouter.reset(`/portal/custom/edit/${kind}/thing`);
      arrange();

      render(<Component />, { wrapper });

      expect(await shownDocument()).toMatchObject(shown);
    }
  );
});
