vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});
vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return {
    ...actual,
    putDefinition: vi.fn(),
    putDataset: vi.fn(),
    fetchMetricNames: vi.fn(),
    fetchWidgetNames: vi.fn(),
    fetchDashboardNames: vi.fn(),
    fetchDatasetNames: vi.fn(),
    fetchDataset: vi.fn(),
    fetchTables: vi.fn(),
    fetchTable: vi.fn(),
  };
});

import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";

import { invalid, mockCatalogues, rowOf, wrapper } from "./editor-test-helpers";
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
  it("puts a refusal on the field it names, and nowhere else", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDefinition).mockRejectedValue(
      invalid([{ field: "limit", description: "must be positive" }])
    );

    render(
      <DefinitionEditor
        kind="metrics"
        name="wrong"
        document={{ dataset: "commits", limit: -1 }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    const said = await screen.findByRole("alert");
    expect(said).toHaveTextContent("must be positive");
    expect(rowOf(screen.getByLabelText("Limit"))).toContainElement(said);
  });

  it("puts each of several refusals on its own field", async () => {
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
    expect(alerts).toHaveLength(2);
    expect(rowOf(screen.getByLabelText("Title"))).toHaveTextContent(
      "must not be empty"
    );
    expect(screen.getByRole("group", { name: "field 2" })).toHaveTextContent(
      "unknown type"
    );
    expect(
      screen.getByRole("group", { name: "field 1" })
    ).not.toHaveTextContent("unknown type");
  });

  it("puts a refusal about a whole entry on that entry", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDataset).mockRejectedValue(
      invalid([{ field: "fields[0]", description: "a field is an object" }])
    );

    render(
      <DefinitionEditor
        kind="datasets"
        name="commits"
        document={{ title: "Commits", fields: [{ name: "day" }] }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByRole("group", { name: "field 1" })
    ).toHaveTextContent("a field is an object");
  });

  it("says a refusal about a field the chosen variant does not show", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDefinition).mockRejectedValue(
      invalid([{ field: "columns", description: "unknown column" }])
    );

    render(
      <DefinitionEditor
        kind="widgets"
        name="chart"
        document={{ type: "stat", metric: "commits", value: "total" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "columns: unknown column"
    );
  });

  it("says a refusal it has no field for, above the document", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDefinition).mockRejectedValue(
      new CustomApiError(409, { detail: "that name is taken" })
    );

    render(
      <DefinitionEditor
        kind="metrics"
        name="taken"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    const said = await screen.findByText("that name is taken");
    const document = screen.getByLabelText("Dataset");
    expect(
      said.compareDocumentPosition(document) & Node.DOCUMENT_POSITION_FOLLOWING
    ).toBeTruthy();
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

  it("shows a placed refusal in the text as well, by its path", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDefinition).mockRejectedValue(
      invalid([{ field: "limit", description: "must be positive" }])
    );

    render(
      <DefinitionEditor
        kind="metrics"
        name="wrong"
        document={{ dataset: "commits", limit: -1 }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));
    await screen.findByRole("alert");
    await user.click(screen.getByRole("button", { name: "JSON" }));

    expect(screen.getByRole("alert")).toHaveTextContent(
      "limit: must be positive"
    );
  });

  // Two main dates is a refusal; the form offers only what a declaration
});
