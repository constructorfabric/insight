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

import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";

import {
  mockCatalogues,
  offeredBy,
  showText,
  wrapper,
} from "./editor-test-helpers";
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

describe("<DefinitionEditor> over a kind's shape", () => {
  it("adds and removes the entries of a list", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });

    await user.click(screen.getByRole("button", { name: "Add field" }));

    const first = within(screen.getByRole("group", { name: "field 1" }));
    await user.type(first.getByLabelText("Name"), "author");

    expect(first.getByLabelText("Path")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /remove field 1/i }));

    expect(
      screen.queryByRole("group", { name: "field 1" })
    ).not.toBeInTheDocument();
  });

  it("keeps an entry of a list that was emptied", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="widgets"
        name="chart"
        document={{
          type: "table",
          metric: "commits",
          columns: ["day", "lines"],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.clear(screen.getByLabelText("column 1"));

    expect(screen.getByLabelText("column 2")).toHaveValue("lines");
  });

  // The document's key beside every label ties a row to the text view and to
  // the path a refusal names.
  it("shows each row's document key in brackets after its label", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });
    await user.click(screen.getByRole("button", { name: "Add field" }));

    const first = screen.getByRole("group", { name: "field 1" });
    expect(first).toHaveTextContent(/Name\s*\(name\)\s*\*/);
    expect(first).toHaveTextContent(/Main date\s*\(default_clock\)/);
  });

  it("names a field once, whatever it is edited with", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });

    await user.click(screen.getByRole("button", { name: "Add field" }));

    const first = within(screen.getByRole("group", { name: "field 1" }));
    expect(first.getAllByText("Main date")).toHaveLength(1);
    expect(first.getByLabelText("Main date")).not.toBeChecked();
  });

  it("asks a widget only for what its type draws", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="widgets"
        name="chart"
        document={{ type: "table", metric: "commits", columns: ["day"] }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    expect(screen.getByLabelText("column 1")).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("Type"), "stat");

    expect(screen.queryByLabelText("column 1")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Value")).toBeInTheDocument();
  });

  it("drops what only the old type asked for when the type changes", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="widgets"
        name="chart"
        document={{ type: "table", metric: "commits", columns: ["day"] }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.selectOptions(screen.getByLabelText("Type"), "stat");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "widgets",
        "chart",
        { type: "stat", metric: "commits" }
      )
    );
  });

  it("keeps a dashboard item the kind it is when its name is cleared", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="dashboards"
        name="board"
        document={{ title: "Board", items: [{ widget: "chart" }] }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.clear(screen.getByLabelText("Widget"));

    expect(screen.getByLabelText("Widget")).toBeInTheDocument();
  });

  it("changes what a dashboard item asks for when its kind changes", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="dashboards"
        name="board"
        document={{ title: "Board", items: [{ widget: "chart" }] }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.selectOptions(screen.getByLabelText("Kind"), "heading");
    await user.type(screen.getByLabelText("Heading"), "Flow");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "dashboards",
        "board",
        { title: "Board", items: [{ heading: "Flow" }] }
      )
    );
  });

  it("builds a declaration from fields, a main date and a row identity", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });

    await user.type(screen.getByLabelText("Name"), "commits");
    await user.type(screen.getByLabelText("Title"), "Commits");

    await user.click(screen.getByRole("button", { name: "Add field" }));
    const day = within(screen.getByRole("group", { name: "field 1" }));
    await user.type(day.getByLabelText("Name"), "day");
    await user.type(day.getByLabelText("Path"), "committed_at");
    await user.selectOptions(day.getByLabelText("Type"), "datetime");
    await user.selectOptions(day.getByLabelText("Role"), "time");
    await user.click(day.getByLabelText("Main date"));

    await user.click(screen.getByRole("button", { name: "Add field" }));
    const author = within(screen.getByRole("group", { name: "field 2" }));
    await user.type(author.getByLabelText("Name"), "author");
    await user.type(author.getByLabelText("Path"), "author.email");
    await user.selectOptions(author.getByLabelText("Type"), "string");

    await user.click(screen.getByRole("button", { name: "Add field name" }));
    const identity = screen.getByLabelText("field name 1");
    expect(offeredBy(identity)).toEqual(["day", "author"]);
    await user.type(identity, "author");

    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDataset).toHaveBeenCalledWith("commits", {
        title: "Commits",
        fields: [
          {
            name: "day",
            path: "committed_at",
            type: "datetime",
            role: "time",
            default_clock: true,
          },
          { name: "author", path: "author.email", type: "string" },
        ],
        row_identity: ["author"],
      })
    );
  });

  it("moves the main date rather than letting two fields carry it", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="datasets"
        name="commits"
        document={{
          title: "Commits",
          fields: [
            { name: "day", path: "day", type: "datetime", default_clock: true },
            { name: "merged", path: "merged", type: "datetime" },
          ],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const first = within(screen.getByRole("group", { name: "field 1" }));
    const second = within(screen.getByRole("group", { name: "field 2" }));
    await user.click(second.getByLabelText("Main date"));

    expect(second.getByLabelText("Main date")).toBeChecked();
    expect(first.getByLabelText("Main date")).not.toBeChecked();
  });

  it("sends a filter's value as the type the filter names", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        name="big"
        document={{
          dataset: "commits",
          fields: [{ field: "lines", type: "int", as_name: "lines" }],
          filters: [{ field: "lines", type: "int", op: "gt" }],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const filter = within(screen.getByRole("group", { name: "filter 1" }));
    await user.type(filter.getByLabelText("Against"), "500");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "big",
        expect.objectContaining({
          filters: [{ field: "lines", type: "int", op: "gt", value: 500 }],
        })
      )
    );
  });

  // The service compares against the declared type: a filter over a `bool`
  // field takes `true`, not `"true"`, whatever the filter's own type says.
  it("compares a filter as the type the dataset declares for its field", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchDataset).mockResolvedValue({
      name: "commits",
      declaration: {
        title: "Commits",
        fields: [
          { name: "merged", path: "merged", type: "bool" },
          { name: "lines", path: "lines", type: "int" },
        ],
      },
    });

    render(
      <DefinitionEditor
        kind="metrics"
        name="merged_only"
        document={{
          dataset: "commits",
          fields: [{ type: "int", agg: "count", as_name: "n" }],
          filters: [{ field: "merged", type: "string", op: "eq", value: true }],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const filter = within(screen.getByRole("group", { name: "filter 1" }));
    const against = await filter.findByRole("combobox", { name: "Against" });
    expect(against).toHaveValue("true");

    await user.selectOptions(against, "false");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "merged_only",
        expect.objectContaining({
          filters: [
            { field: "merged", type: "string", op: "eq", value: false },
          ],
        })
      )
    );
  });

  it("offers the dataset's declared fields to read and to filter by", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchDataset).mockResolvedValue({
      name: "commits",
      declaration: {
        title: "Commits",
        fields: [
          { name: "day", path: "day", type: "datetime" },
          { name: "author", path: "author", type: "string" },
        ],
      },
    });

    render(
      <DefinitionEditor
        kind="metrics"
        name="by_author"
        document={{
          dataset: "commits",
          fields: [{ type: "int", agg: "count", as_name: "n" }],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const field = within(screen.getByRole("group", { name: "field 1" }));
    await waitFor(() =>
      expect(offeredBy(field.getByLabelText("Reads"))).toEqual([
        "day",
        "author",
      ])
    );

    const filtered = within(screen.getByRole("group", { name: "Filtered" }));
    await user.click(filtered.getByRole("button", { name: "Add filter" }));
    const filter = within(screen.getByRole("group", { name: "filter 1" }));
    expect(offeredBy(filter.getByLabelText("Field"))).toEqual([
      "day",
      "author",
    ]);
  });

  it("offers a metric's own columns, and the bucket, to group by", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        name="grouped"
        document={{
          dataset: "commits",
          fields: [
            { field: "author", type: "string", as_name: "who" },
            { type: "int", agg: "count", as_name: "how_many" },
          ],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const grouped = within(screen.getByRole("group", { name: "Grouped by" }));
    await user.click(grouped.getByRole("button", { name: "Add column" }));

    expect(offeredBy(grouped.getByLabelText("column 1"))).toEqual([
      "who",
      "how_many",
      "bucket",
    ]);
  });

  it("asks a metric which it reads, and offers a table's columns over one", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchTables).mockResolvedValue({
      tables: [{ database: "silver", table: "fct_commit", layer: "silver" }],
      total: 1,
    });
    vi.mocked(customClient.fetchTable).mockResolvedValue({
      database: "silver",
      table: "fct_commit",
      layer: "silver",
      engine: "MergeTree",
      columns: [
        { name: "sha", type: "String" },
        { name: "lines_added", type: "Int64" },
      ],
    });
    render(<DefinitionEditor kind="metrics" onStored={vi.fn()} />, {
      wrapper,
    });

    const reads = screen.getByLabelText(/Source/);
    expect(
      [...reads.querySelectorAll("option")].map((option) => option.value)
    ).toEqual(expect.arrayContaining(["dataset", "table"]));
    await user.selectOptions(reads, "table");

    const table = screen.getByLabelText(/^Table/);
    await waitFor(() =>
      expect(offeredBy(table)).toEqual(["silver.fct_commit"])
    );
    await user.type(table, "silver.fct_commit");
    await user.click(screen.getByRole("button", { name: "Add field" }));
    const first = within(screen.getByRole("group", { name: "field 1" }));
    await waitFor(() =>
      expect(offeredBy(first.getByLabelText(/^Column/))).toEqual([
        "sha",
        "lines_added",
      ])
    );
    expect(first.getByLabelText(/JSON key/)).toBeInTheDocument();
    expect(first.queryByLabelText(/^Field/)).not.toBeInTheDocument();
    expect(customClient.fetchTable).toHaveBeenCalledWith("silver", "fct_commit");
  });

  it("finds a bare table's database in the catalogue when only one has it", async () => {
    vi.mocked(customClient.fetchTables).mockResolvedValue({
      tables: [{ database: "bronze_github", table: "issues", layer: "bronze" }],
      total: 1,
    });
    render(
      <DefinitionEditor
        kind="metrics"
        name="open_issues"
        document={{
          table: "issues",
          fields: [{ column: "number", type: "int", agg: "count", as_name: "n" }],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await waitFor(() =>
      expect(customClient.fetchTable).toHaveBeenCalledWith(
        "bronze_github",
        "issues"
      )
    );
  });

  it("reads a stored metric over a table as one, with the dataset rows out of sight", () => {
    render(
      <DefinitionEditor
        kind="metrics"
        name="lines"
        document={{
          database: "silver",
          table: "fct_commit",
          fields: [
            { column: "lines_added", type: "int", agg: "sum", as_name: "lines" },
          ],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    expect(screen.getByLabelText(/Source/)).toHaveValue("table");
    expect(screen.getByLabelText(/^Table/)).toHaveValue("fct_commit");
    expect(screen.getByLabelText(/^Database/)).toHaveValue("silver");
    expect(screen.queryByLabelText(/^Dataset/)).not.toBeInTheDocument();
  });

  it("drops the dataset name when a metric is turned over a table, and keeps its fields", async () => {
    const user = userEvent.setup();
    render(
      <DefinitionEditor
        kind="metrics"
        name="per_actor"
        document={{
          dataset: "commits",
          fields: [{ field: "actor", type: "string", as_name: "actor" }],
          group_by: ["actor"],
        }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.selectOptions(screen.getByLabelText(/Source/), "table");
    const text = await showText(user);

    const sent = JSON.parse(text.value) as Record<string, unknown>;
    expect(sent).not.toHaveProperty("dataset");
    expect(sent).toHaveProperty("table", "");
    expect(sent).toHaveProperty("group_by", ["actor"]);
  });
});
