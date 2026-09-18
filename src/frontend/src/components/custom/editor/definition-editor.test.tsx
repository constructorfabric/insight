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
  };
});

import { render, screen, waitFor } from "@testing-library/react";
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

describe("<DefinitionEditor> holding one document", () => {
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

  // A rename rewrites everything that pointed at the old name, so it is its
  // own action; a field that looked editable and was not read as broken.
  it("shows an existing definition's name as text, and says where it is renamed", () => {
    render(
      <DefinitionEditor
        kind="metrics"
        name="already_there"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    expect(
      screen.queryByRole("textbox", { name: "Name" })
    ).not.toBeInTheDocument();
    expect(screen.getByText("already_there")).toBeInTheDocument();
    expect(screen.getByText(/Renamed under Rename, below/)).toBeInTheDocument();
  });

  it("says that a dataset keeps its name", () => {
    render(
      <DefinitionEditor
        kind="datasets"
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

  // The service replaces on write: a new definition under a taken name would
  // write over the one that exists.
  it("refuses to create under a name the catalogue already holds", async () => {
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

  it("does not mistake an existing definition for a clash with itself", async () => {
    vi.mocked(customClient.fetchMetricNames).mockResolvedValue({
      names: ["lines_per_day"],
      total: 1,
    });

    render(
      <DefinitionEditor
        kind="metrics"
        name="lines_per_day"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await waitFor(() =>
      expect(customClient.fetchMetricNames).toHaveBeenCalled()
    );
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
        name="ready"
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
});
