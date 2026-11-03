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

import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { datasetNames } from "@/test/catalogue";

import { mockCatalogues, wrapper, writeText } from "./editor-test-helpers";
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
    await writeText(user, '{"dataset": "commits", "limit": 100}');
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

  it("begins a new definition as an empty object", () => {
    render(<DefinitionEditor kind="widgets" onStored={vi.fn()} />, {
      wrapper,
    });

    expect(screen.getByLabelText(/as text/i)).toHaveValue("{}");
  });

  it("shows a stored definition as its text", () => {
    render(
      <DefinitionEditor
        kind="metrics"
        id="m1"
        name="Stored"
        document={{ dataset: "commits", limit: 5 }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    const text = screen.getByLabelText(/as text/i) as HTMLTextAreaElement;
    expect(JSON.parse(text.value)).toEqual({ dataset: "commits", limit: 5 });
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

    await writeText(user, "{not json");

    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });

  it("sends every property the text holds", async () => {
    const user = userEvent.setup();

    render(
      <DefinitionEditor
        kind="metrics"
        id="kept"
        name="Kept"
        document={{ dataset: "commits" }}
        onStored={vi.fn()}
      />,
      { wrapper }
    );

    await writeText(
      user,
      '{"dataset": "commits", "something_new": 7, "limit": 5}'
    );
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
    await writeText(user, '{"title": "Commits"}');
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(customClient.putDataset).toHaveBeenCalledWith("commits", {
        title: "Commits",
      })
    );
  });

  it("refuses to create a dataset under a name the catalogue already holds", async () => {
    const user = userEvent.setup();
    vi.mocked(customClient.fetchDatasetNames).mockResolvedValue(
      datasetNames(["commits"])
    );

    render(<DefinitionEditor kind="datasets" onStored={vi.fn()} />, {
      wrapper,
    });

    await user.type(screen.getByLabelText("Name"), "commits");

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

  it("lets a new metric take a name another metric already has", async () => {
    const user = userEvent.setup();

    render(<DefinitionEditor kind="metrics" onStored={vi.fn()} />, { wrapper });

    await user.type(screen.getByLabelText("Name"), "Lines per day");
    await writeText(user, '{"dataset": "commits"}');

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
});
