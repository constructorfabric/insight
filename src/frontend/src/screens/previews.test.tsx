/** The previews screen refuses ungated viewers and drives the console. */
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Experiment } from "@/api/previews-client";

const gate = vi.hoisted(() => ({ allowed: true }));
const hooks = vi.hoisted(() => ({
  experiments: {
    isPending: false,
    isError: false,
    data: [] as Experiment[],
    refetch: vi.fn(),
  },
  create: {
    isPending: false,
    isError: false,
    error: null as unknown,
    mutate: vi.fn(),
  },
  remove: {
    isPending: false,
    isError: false,
    error: null as unknown,
    mutate: vi.fn(),
    reset: vi.fn(),
  },
}));
vi.mock("@/queries/previews", () => ({
  usePreviewsGate: () => gate.allowed,
  useExperiments: () => hooks.experiments,
  useCreateExperiment: () => hooks.create,
  useDeleteExperiment: () => hooks.remove,
}));

import { PreviewsBody } from "./previews";

const EXPERIMENT: Experiment = {
  name: "my-experiment",
  tag: "preview-my-branch",
  url: "https://preview.example.com/exp/my-experiment/",
  creator: "00000000-0000-0000-0000-000000000001",
  status: "ready",
};

beforeEach(() => {
  vi.clearAllMocks();
  gate.allowed = true;
  hooks.experiments.data = [];
  hooks.experiments.isPending = false;
  hooks.experiments.isError = false;
});

describe("PreviewsBody", () => {
  it("refuses a viewer the gate does not pass — no form, no listing", () => {
    gate.allowed = false;

    render(<PreviewsBody />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Previews are not available",
    );
    expect(
      screen.queryByRole("form", { name: "Create a preview experiment" }),
    ).not.toBeInTheDocument();
  });

  it("renders the listing with an open link to the served URL", () => {
    hooks.experiments.data = [EXPERIMENT];

    render(<PreviewsBody />);

    expect(screen.getByText("my-experiment")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Open experiment my-experiment" }),
    ).toHaveAttribute("href", EXPERIMENT.url);
  });

  it("delete asks for confirmation before mutating", () => {
    hooks.experiments.data = [EXPERIMENT];

    render(<PreviewsBody />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    expect(hooks.remove.mutate).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getAllByRole("button", { name: "Delete" }).at(-1)!,
    );
    expect(hooks.remove.mutate).toHaveBeenCalledWith(
      "my-experiment",
      expect.anything(),
    );
  });

  it("the create form does not submit while a field is empty", () => {
    render(<PreviewsBody />);

    const submit = screen.getByRole("button", { name: /Create/ });
    expect(submit).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "my-experiment" },
    });
    expect(submit).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Image tag"), {
      target: { value: "preview-my-branch" },
    });
    expect(submit).toBeEnabled();

    fireEvent.click(submit);
    expect(hooks.create.mutate).toHaveBeenCalledWith(
      { name: "my-experiment", tag: "preview-my-branch" },
      expect.anything(),
    );
  });
});
