vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    createDefinition: vi.fn(),
    putDefinition: vi.fn(),
    putDataset: vi.fn(),
    fetchMetric: vi.fn(),
    fetchMetrics: vi.fn(),
    fetchWidget: vi.fn(),
    fetchWidgets: vi.fn(),
    fetchDashboards: vi.fn(),
    fetchDatasetNames: vi.fn(),
    fetchDataset: vi.fn(),
  };
});

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";

import { invalid, mockCatalogues, wrapper } from "./editor-test-helpers";
import { DefinitionEditor } from "./definition-editor";

beforeEach(() => {
  vi.resetAllMocks();
  mockCatalogues();
  vi.mocked(customClient.putDefinition).mockResolvedValue(undefined);
  vi.mocked(customClient.putDataset).mockResolvedValue({
    name: "x",
    declaration: { title: "x", fields: [] },
  });
});

describe("<DefinitionEditor> shown a refusal", () => {
  it("says each refusal under the document, by the path it names", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDataset).mockRejectedValue(
      invalid([
        { field: "title", description: "must not be empty" },
        { field: "fields[1].type", description: "unknown type" },
      ])
    );

    render(
      <DefinitionEditor
        kind="datasets"
        id="commits"
        name="commits"
        document={{
          title: "",
          fields: [
            { name: "day", path: "day", type: "datetime" },
            { name: "n", path: "n", type: "moment" },
          ],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    const alerts = await screen.findAllByRole("alert");
    expect(alerts.map((alert) => alert.textContent)).toEqual([
      "title: must not be empty",
      "fields[1].type: unknown type",
    ]);
  });

  it("says the service's own words when it names nothing", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDefinition).mockRejectedValue(
      new CustomApiError(409, { detail: "the metric changed meanwhile" })
    );

    render(
      <DefinitionEditor
        kind="metrics"
        id="taken"
        name="taken"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "the metric changed meanwhile"
    );
  });

  it("names what a change would break, when the service refuses for that", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDataset).mockRejectedValue(
      new CustomApiError(409, {
        detail: "Operation precondition not met",
        context: {
          violations: [
            {
              type: "would_break",
              subject: "lines_per_day",
              description: "reads `lines`, which is gone",
            },
          ],
        },
      })
    );

    render(
      <DefinitionEditor
        kind="datasets"
        id="commits"
        name="commits"
        document={{ title: "Commits", fields: [] }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "lines_per_day: reads `lines`, which is gone"
    );
  });

  it("says so when the failure is not the service's refusal", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDefinition).mockRejectedValue(
      new TypeError("Failed to fetch")
    );

    render(
      <DefinitionEditor
        kind="metrics"
        id="offline"
        name="offline"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't store it."
    );
  });
});
