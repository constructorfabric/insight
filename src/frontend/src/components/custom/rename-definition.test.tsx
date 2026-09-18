vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, renameDefinition: vi.fn() };
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";

import { RenameDefinition } from "./rename-definition";

function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

beforeEach(() => {
  vi.mocked(customClient.renameDefinition).mockReset();
});

describe("<RenameDefinition>", () => {
  it("sends the name the reader typed", async () => {
    vi.mocked(customClient.renameDefinition).mockResolvedValue({
      name: "commits_daily",
      rewritten: ["commits_table"],
    });

    render(<RenameDefinition kind="metrics" name="commits_per_day" />, {
      wrapper,
    });

    await userEvent.click(
      screen.getByRole("button", { name: "Rename commits_per_day" })
    );
    const field = screen.getByRole("textbox", {
      name: "New name for commits_per_day",
    });
    await userEvent.clear(field);
    await userEvent.type(field, "commits_daily");
    await userEvent.click(screen.getByRole("button", { name: "Rename" }));

    expect(customClient.renameDefinition).toHaveBeenCalledWith(
      "metrics",
      "commits_per_day",
      "commits_daily"
    );
  });

  it("starts from the name it has, so a rename is an edit", async () => {
    render(<RenameDefinition kind="widgets" name="commits_table" />, {
      wrapper,
    });

    await userEvent.click(
      screen.getByRole("button", { name: "Rename commits_table" })
    );

    expect(
      screen.getByRole("textbox", { name: "New name for commits_table" })
    ).toHaveValue("commits_table");
  });

  it("asks for nothing when the name has not changed", async () => {
    render(<RenameDefinition kind="dashboards" name="engineering" />, {
      wrapper,
    });

    await userEvent.click(
      screen.getByRole("button", { name: "Rename engineering" })
    );
    await userEvent.click(screen.getByRole("button", { name: "Rename" }));

    expect(customClient.renameDefinition).not.toHaveBeenCalled();
  });

  it("backs out without renaming", async () => {
    render(<RenameDefinition kind="metrics" name="commits_per_day" />, {
      wrapper,
    });

    await userEvent.click(
      screen.getByRole("button", { name: "Rename commits_per_day" })
    );
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(customClient.renameDefinition).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "Rename commits_per_day" })
    ).toBeInTheDocument();
  });

  it("shows a taken name as the service refused it", async () => {
    // An already_exists refusal carries its message in `detail`.
    vi.mocked(customClient.renameDefinition).mockRejectedValue(
      new customClient.CustomApiError(409, {
        detail: "`already_here` is already taken",
      })
    );

    render(<RenameDefinition kind="metrics" name="commits_per_day" />, {
      wrapper,
    });
    await userEvent.click(
      screen.getByRole("button", { name: "Rename commits_per_day" })
    );
    const field = screen.getByRole("textbox", {
      name: "New name for commits_per_day",
    });
    await userEvent.clear(field);
    await userEvent.type(field, "already_here");
    await userEvent.click(screen.getByRole("button", { name: "Rename" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "`already_here` is already taken"
    );
  });
});
