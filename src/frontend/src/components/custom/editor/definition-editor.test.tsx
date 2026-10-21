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
    fetchMetrics: vi.fn(),
    fetchWidgets: vi.fn(),
    fetchDashboards: vi.fn(),
    fetchDatasetNames: vi.fn(),
    fetchDataset: vi.fn(),
    fetchTables: vi.fn(),
    fetchTable: vi.fn(),
  };
});

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";

import {
  labelsOfferedBy,
  mockCatalogues,
  offeredBy,
  showText,
  wrapper,
} from "./editor-test-helpers";
import { DefinitionEditor } from "./definition-editor";

beforeEach(() => {
  vi.resetAllMocks();
  mockCatalogues();
  vi.mocked(customClient.createDefinition).mockResolvedValue("m-new");
  vi.mocked(customClient.putDefinition).mockResolvedValue(undefined);
  vi.mocked(customClient.putDataset).mockResolvedValue({
    name: "x",
    declaration: { title: "x", fields: [] },
  });
});

describe("<DefinitionEditor> holding one document", () => {
  it("creates a new definition from what was written, and hands back its id", async () => {
    const user = userEvent.setup();
    const stored = vi.fn();

    render(<DefinitionEditor kind="metrics" onStored={stored} />, { wrapper });

    await user.type(screen.getByLabelText("Name"), "Lines per day");
    await user.type(screen.getByLabelText("Dataset"), "commits");
    await user.type(screen.getByLabelText("Limit"), "100");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.createDefinition).toHaveBeenCalledWith(
        "metrics",
        "Lines per day",
        { dataset: "commits", limit: 100 }
      )
    );
    expect(customClient.putDefinition).not.toHaveBeenCalled();
    await waitFor(() => expect(stored).toHaveBeenCalledWith("m-new"));
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
    await user.click(screen.getByRole("button", { name: "Editor" }));

    expect(screen.getByLabelText("Dataset")).toHaveValue("pull_requests");
  });

  it("will not send text that does not parse", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        id="broken"
        name="Broken"
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const text = await showText(user);
    await user.clear(text);
    await user.type(text, "{{not json");

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });

  it("sends a property it has no field for", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        id="kept"
        name="Kept"
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
        "Kept",
        {
          dataset: "commits",
          something_new: 7,
          limit: 5,
        }
      )
    );
  });

  // Nothing points at a definition by its name, so a rename is an ordinary save.
  it("renames an existing definition by saving it under the new name", async () => {
    const user = userEvent.setup();
    const stored = vi.fn();

    render(
      <DefinitionEditor
        kind="metrics"
        id="m1"
        name="Already there"
        document={{ dataset: "commits" }}
        onStored={stored}
      />,
      { wrapper }
    );

    const name = screen.getByRole("textbox", { name: "Name" });
    expect(name).toHaveValue("Already there");
    expect(name).toHaveAttribute("maxLength", "128");
    await user.clear(name);
    await user.type(name, "Renamed");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "m1",
        "Renamed",
        { dataset: "commits" }
      )
    );
    await waitFor(() => expect(stored).toHaveBeenCalledWith("m1"));
  });

  it("says that a dataset keeps its name", () => {
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

    expect(screen.getByText(/A dataset keeps its name/)).toBeInTheDocument();
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

  it("sends no number at all for one that was not written", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        id="unlimited"
        name="Unlimited"
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
        "Unlimited",
        { dataset: "commits" }
      )
    );
  });

  // A dataset is written under its name: a new one under a taken name would
  // write over the one that exists.
  it("refuses to create a dataset under a name the catalogue already holds", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits"],
      total: 1,
    });

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });
    await screen
      .findByText("", { selector: "option[value='commits']" })
      .catch(() => undefined);

    await user.type(screen.getByLabelText("Name"), "commits");
    await user.type(screen.getByLabelText("Title"), "Commits");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "A dataset called commits already exists."
    );
    expect(screen.getByRole("link", { name: "Open it" })).toHaveAttribute(
      "href",
      "/portal/custom/edit/datasets/commits"
    );
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await user.type(screen.getByLabelText("Name"), "_2");

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  });

  // Names are labels: two metrics may share one.
  it("lets a new metric take a name another metric already has", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [{ id: "m1", name: "Lines per day" }],
      total: 1,
    });

    render(<DefinitionEditor kind="metrics" onStored={vi.fn()} />, { wrapper });

    await waitFor(() => expect(customClient.fetchMetrics).toHaveBeenCalled());
    await user.type(screen.getByLabelText("Name"), "Lines per day");
    await user.type(screen.getByLabelText("Dataset"), "commits");

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("stores nothing until Save is pressed", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });

    await user.type(screen.getByLabelText("Name"), "commits{Enter}");

    expect(customClient.putDataset).not.toHaveBeenCalled();
  });

  it("stores when Enter is pressed on Save itself", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        id="ready"
        name="Ready"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    screen.getByRole("button", { name: "Save" }).focus();
    await user.keyboard("{Enter}");

    await waitFor(() =>
      expect(customClient.putDefinition).toHaveBeenCalledWith(
        "metrics",
        "ready",
        "Ready",
        { dataset: "commits" }
      )
    );
  });

  it("offers the names already stored for a reference", async () => {
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue({
      names: ["commits", "pull_requests"],
      total: 2,
    });

    render(<DefinitionEditor kind="metrics" onStored={vi.fn()} />, { wrapper });

    await screen.findByText("", { selector: "option[value='commits']" });
    expect(offeredBy(screen.getByLabelText("Dataset"))).toEqual([
      "commits",
      "pull_requests",
    ]);
  });

  it("offers a reference by id, shown under its name, and names the one chosen", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchMetrics).mockResolvedValue({
      items: [
        { id: "0192f0aa-0000-7000-8000-000000000001", name: "Commits" },
        { id: "legacy_metric", name: "legacy_metric" },
      ],
      total: 2,
    });

    render(
      <DefinitionEditor
        kind="widgets"
        id="w1"
        name="Commits table"
        document={{ type: "table", metric: "", columns: [] }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const metric = screen.getByLabelText("Metric");
    await waitFor(() =>
      expect(offeredBy(metric)).toEqual([
        "0192f0aa-0000-7000-8000-000000000001",
        "legacy_metric",
      ])
    );
    expect(labelsOfferedBy(metric)).toEqual(["Commits", ""]);

    await user.type(metric, "0192f0aa-0000-7000-8000-000000000001");

    expect(screen.getByText("Commits", { selector: "span" })).toBeInTheDocument();
  });
});
