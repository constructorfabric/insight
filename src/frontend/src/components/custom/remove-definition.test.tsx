vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, deleteDefinition: vi.fn() };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";

import { RemoveDefinition } from "./remove-definition";

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

beforeEach(() => {
  vi.mocked(customClient.deleteDefinition).mockReset();
});

describe("<RemoveDefinition>", () => {
  it("asks before it removes", async () => {
    vi.mocked(customClient.deleteDefinition).mockResolvedValue(undefined);

    render(<RemoveDefinition kind="metrics" name="lines_per_day" />, {
      wrapper,
    });

    await userEvent.click(
      screen.getByRole("button", { name: "Remove lines_per_day" })
    );
    // Nothing is gone until the second click.
    expect(customClient.deleteDefinition).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(customClient.deleteDefinition).toHaveBeenCalledWith(
      "metrics",
      "lines_per_day"
    );
  });

  it("keeps it when the reader backs out", async () => {
    render(<RemoveDefinition kind="widgets" name="lines_table" />, { wrapper });

    await userEvent.click(
      screen.getByRole("button", { name: "Remove lines_table" })
    );
    await userEvent.click(screen.getByRole("button", { name: "Keep" }));

    expect(customClient.deleteDefinition).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "Remove lines_table" })
    ).toBeInTheDocument();
  });

  it("shows what still uses it, as the service said it", async () => {
    // The shape a live refusal carries: context.violations[].description.
    vi.mocked(customClient.deleteDefinition).mockRejectedValue(
      new customClient.CustomApiError(400, {
        context: {
          violations: [{ description: "still in use by lines_table" }],
        },
      })
    );

    render(<RemoveDefinition kind="metrics" name="lines_per_day" />, {
      wrapper,
    });
    await userEvent.click(
      screen.getByRole("button", { name: "Remove lines_per_day" })
    );
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "still in use by lines_table"
    );
  });

  it("falls back to a plain message when the failure says nothing", async () => {
    vi.mocked(customClient.deleteDefinition).mockRejectedValue(
      new Error("network down")
    );

    render(<RemoveDefinition kind="dashboards" name="engineering" />, {
      wrapper,
    });
    await userEvent.click(
      screen.getByRole("button", { name: "Remove engineering" })
    );
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't remove it."
    );
  });
});
