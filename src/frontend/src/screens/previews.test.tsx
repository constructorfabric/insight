/** The previews screen refuses ungated viewers and drives the console. */
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Experiment } from "@/api/previews-client";

const gate = vi.hoisted(() => ({ allowed: true }));
const hooks = vi.hoisted(() => ({
  experiments: {
    isPending: false,
    isError: false,
    data: { experiments: [] as Experiment[], liveCount: 0, cap: 10 },
    refetch: vi.fn(),
  },
  images: {
    isPending: false,
    isError: false,
    data: { configured: false, tags: [] as string[] } as
      { configured: boolean; tags: string[] } | undefined,
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
  useImages: () => hooks.images,
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
  hooks.experiments.data = { experiments: [], liveCount: 0, cap: 10 };
  hooks.experiments.isPending = false;
  hooks.experiments.isError = false;
  hooks.images.data = { configured: false, tags: [] };
  hooks.images.isError = false;
});

describe("PreviewsBody", () => {
  it("refuses a viewer the gate does not pass — no form, no listing", () => {
    gate.allowed = false;

    render(<PreviewsBody />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Previews are not available"
    );
    expect(
      screen.queryByRole("form", { name: "Create a preview experiment" })
    ).not.toBeInTheDocument();
  });

  it("renders the listing with an open link to the served URL", () => {
    hooks.experiments.data = {
      experiments: [EXPERIMENT],
      liveCount: 1,
      cap: 10,
    };

    render(<PreviewsBody />);

    expect(screen.getByText("my-experiment")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Open experiment my-experiment" })
    ).toHaveAttribute("href", EXPERIMENT.url);
  });

  it("shows the live count against the cap", () => {
    hooks.experiments.data = {
      experiments: [EXPERIMENT],
      liveCount: 1,
      cap: 5,
    };

    render(<PreviewsBody />);

    expect(screen.getByText("1 of 5 experiments")).toBeInTheDocument();
  });

  it("delete asks for confirmation before mutating", () => {
    hooks.experiments.data = {
      experiments: [EXPERIMENT],
      liveCount: 1,
      cap: 10,
    };

    render(<PreviewsBody />);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    expect(hooks.remove.mutate).not.toHaveBeenCalled();
    fireEvent.click(screen.getAllByRole("button", { name: "Delete" }).at(-1)!);
    expect(hooks.remove.mutate).toHaveBeenCalledWith(
      "my-experiment",
      expect.anything()
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
      expect.anything()
    );
  });

  it("a filled TTL rides the create request; empty means the server default", () => {
    render(<PreviewsBody />);

    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "my-experiment" },
    });
    fireEvent.change(screen.getByLabelText("Image tag"), {
      target: { value: "preview-my-branch" },
    });
    fireEvent.change(screen.getByLabelText("TTL days"), {
      target: { value: "3" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Create/ }));

    expect(hooks.create.mutate).toHaveBeenCalledWith(
      { name: "my-experiment", tag: "preview-my-branch", ttlDays: 3 },
      expect.anything()
    );
  });

  it("suggests registry tags only when the listing is configured and non-empty", () => {
    hooks.images.data = {
      configured: true,
      tags: ["preview-alpha", "preview-beta"],
    };

    const { unmount } = render(<PreviewsBody />);
    expect(screen.getByLabelText("Image tag")).toHaveAttribute(
      "list",
      "previews-tag-options"
    );
    unmount();

    hooks.images.data = { configured: false, tags: [] };
    render(<PreviewsBody />);
    expect(screen.getByLabelText("Image tag")).not.toHaveAttribute("list");
  });

  it("keeps free-text tag entry when the tag listing errors", () => {
    hooks.images.data = undefined;
    hooks.images.isError = true;

    render(<PreviewsBody />);

    const tagInput = screen.getByLabelText("Image tag");
    expect(tagInput).not.toHaveAttribute("list");
    expect(tagInput).toBeEnabled();
  });
});
