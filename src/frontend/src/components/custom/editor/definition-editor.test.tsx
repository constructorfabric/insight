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
  };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";

import { DefinitionEditor } from "./definition-editor";

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

const NO_NAMES = { names: [], total: 0 };

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(customClient.fetchMetricNames).mockResolvedValue(NO_NAMES);
  vi.mocked(customClient.fetchWidgetNames).mockResolvedValue(NO_NAMES);
  vi.mocked(customClient.fetchDashboardNames).mockResolvedValue(NO_NAMES);
  vi.mocked(customClient.fetchDatasetNames).mockResolvedValue(NO_NAMES);
  vi.mocked(customClient.putDefinition).mockResolvedValue(undefined);
  vi.mocked(customClient.putDataset).mockResolvedValue({
    name: "x",
    declaration: { title: "x", fields: [] },
  });
});

function textView(): HTMLTextAreaElement {
  return screen.getByLabelText(/as text/i) as HTMLTextAreaElement;
}

async function showText(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "Text" }));
  return textView();
}

describe("<DefinitionEditor>", () => {
  it("offers the fields the kind admits, and sends what was written", async () => {
    const user = userEvent.setup();
    const stored = vi.fn();

    render(<DefinitionEditor kind="metrics" onStored={stored} />, { wrapper });

    await user.type(screen.getByLabelText("Name"), "lines_per_day");
    await user.type(screen.getByLabelText("Dataset"), "commits");
    await user.type(screen.getByLabelText("Limit"), "100");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "lines_per_day",
        { dataset: "commits", limit: 100 }
      )
    );
    await waitFor(() => expect(stored).toHaveBeenCalledWith("lines_per_day"));
  });

  it("shows what the fields wrote as text, because they are one document", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="metrics" onStored={vi.fn()} />, { wrapper });

    await user.type(screen.getByLabelText("Dataset"), "commits");

    expect(JSON.parse((await showText(user)).value)).toEqual({
      dataset: "commits",
    });
  });

  it("believes the text once it parses, and shows it in the fields", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="metrics" onStored={vi.fn()} />, { wrapper });

    const text = await showText(user);
    await user.clear(text);
    await user.type(text, '{{"dataset": "pull_requests"}');
    await user.click(screen.getByRole("button", { name: "Fields" }));

    expect(screen.getByLabelText("Dataset")).toHaveValue("pull_requests");
  });

  // The document is the thing; the text is a view of it. Half-typed JSON is
  // not a document, so there is nothing to send.
  it("will not send text that does not parse", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor kind="metrics" name="broken" onStored={vi.fn()} />,
      { wrapper }
    );

    const text = await showText(user);
    await user.clear(text);
    await user.type(text, "{{not json");

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });

  // The editor offers what a kind admits; it does not decide what a document
  // may hold, and a property it cannot show is still the author's.
  it("sends a property it has no field for", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        name="kept"
        document={{ dataset: "commits", something_new: 7 }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.type(screen.getByLabelText("Limit"), "5");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "kept",
        {
          dataset: "commits",
          something_new: 7,
          limit: 5,
        }
      )
    );
  });

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

  // A column the old type asked for means nothing to the new one, and sending
  // it would have the service refuse a widget the author never wrote.
  it("drops what the old type asked for when the type changes", async () => {
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
        { type: "stat" }
      )
    );
  });

  it("puts a refusal on the field it names", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.putDefinition).mockRejectedValue(
      new CustomApiError(400, {
        context: {
          violations: [{ field: "dataset", description: "no such dataset" }],
        },
      })
    );

    render(
      <DefinitionEditor
        kind="metrics"
        name="wrong"
        document={{ dataset: "nope" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.click(screen.getByRole("button", { name: "Save" }));

    const said = await screen.findByRole("alert");
    expect(said).toHaveTextContent("no such dataset");
  });

  // The service checks more than the form offers, so a refusal it cannot place
  // is still said rather than leaving a form that refuses in silence.
  it("says a refusal it has no field for", async () => {
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

    expect(await screen.findByText("that name is taken")).toBeInTheDocument();
  });

  // A rename rewrites everything that pointed at the old name; typing over it
  // here would store a second definition instead.
  it("will not rename a definition that already exists", async () => {
    render(
      <DefinitionEditor
        kind="metrics"
        name="already_there"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    expect(screen.getByLabelText("Name")).toHaveAttribute("readonly");
  });

  it("writes a dataset through its own path", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });

    await user.type(screen.getByLabelText("Name"), "commits");
    await user.type(screen.getByLabelText("Title"), "Commits");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDataset).toHaveBeenCalledWith("commits", {
        title: "Commits",
      })
    );
  });

  // A list shortens when an entry is removed, not when what it holds is
  // cleared - otherwise the rows below jump up under the reader's cursor.
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

  // A dashboard item is told apart by the property it carries, so clearing
  // that property would leave an item that is no kind at all.
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

  it("sends no number at all for one that was not written", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        name="unlimited"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await user.type(screen.getByLabelText("Limit"), "1e");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "unlimited",
        { dataset: "commits" }
      )
    );
  });

  // The row carries the label; a control that printed it again would read as
  // two fields with one name.
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

  // Storing claims a name and, for a dataset, builds what holds its records.
  // That is a decision, not something a stray Enter makes.
  it("stores nothing until Save is pressed", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });

    await user.type(screen.getByLabelText("Name"), "commits{Enter}");

    expect(customClient.putDataset).not.toHaveBeenCalled();
  });

  it("offers the names already stored for a reference", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits", "pull_requests"],
      total: 2,
    });

    render(<DefinitionEditor kind="metrics" onStored={vi.fn()} />, { wrapper });

    expect(
      await screen.findByText("", { selector: "option[value='commits']" })
    ).toBeInTheDocument();
  });
});
