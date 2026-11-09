import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import "@/i18n";
import { SourceFilter } from "./source-filter";

const SOURCES = ["github", "gitlab", "gitlab-commit-email"];

function setup(
  picked: readonly string[] = [],
  failed = false,
  sources = SOURCES
) {
  const onChange = vi.fn<(next: string[]) => void>();
  render(
    <SourceFilter
      sources={sources}
      picked={picked}
      failed={failed}
      onChange={onChange}
    />
  );
  return { onChange, user: userEvent.setup() };
}

describe("SourceFilter", () => {
  it("labels the button Sources and never shows a number", () => {
    setup(["github", "gitlab"]);

    const button = screen.getByRole("button", { name: /sources/i });

    expect(button).toHaveTextContent(/^Sources$/);
    expect(button.textContent).not.toMatch(/\d/);
  });

  it("names the picked sources for assistive tech when a filter is active", () => {
    setup(["github", "gitlab"]);

    expect(
      screen.getByRole("button", { name: "Sources: github, gitlab" })
    ).toBeVisible();
  });

  it("says the sources could not be loaded, instead of claiming nothing matches", async () => {
    const { user } = setup([], true, []);

    await user.click(screen.getByRole("button", { name: "Sources" }));

    expect(screen.getByText("Sources could not be loaded")).toBeVisible();
    expect(screen.queryByText("No source matches")).not.toBeInTheDocument();
  });

  it("reports the source that was checked", async () => {
    const { onChange, user } = setup();

    await user.click(screen.getByRole("button", { name: "Sources" }));
    await user.click(screen.getByRole("checkbox", { name: "github" }));

    expect(onChange).toHaveBeenLastCalledWith(["github"]);
  });

  it("adds to the picked set and removes from it", async () => {
    const { onChange, user } = setup(["github"]);

    await user.click(screen.getByRole("button", { name: /sources/i }));
    await user.click(screen.getByRole("checkbox", { name: "gitlab" }));
    expect(onChange).toHaveBeenLastCalledWith(["github", "gitlab"]);

    await user.click(screen.getByRole("checkbox", { name: "github" }));
    expect(onChange).toHaveBeenLastCalledWith([]);
  });

  it("clears the selection from All", async () => {
    const { onChange, user } = setup(["github"]);

    await user.click(screen.getByRole("button", { name: /sources/i }));
    await user.click(screen.getByRole("button", { name: "All" }));

    expect(onChange).toHaveBeenCalledWith([]);
  });

  it("narrows the list by the search box without changing the selection", async () => {
    const { onChange, user } = setup();

    await user.click(screen.getByRole("button", { name: "Sources" }));
    await user.type(
      screen.getByRole("searchbox", { name: "Find a source" }),
      "commit"
    );

    expect(
      screen.getByRole("checkbox", { name: "gitlab-commit-email" })
    ).toBeVisible();
    expect(screen.queryByRole("checkbox", { name: "github" })).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("says so when no source matches", async () => {
    const { user } = setup();

    await user.click(screen.getByRole("button", { name: "Sources" }));
    await user.type(
      screen.getByRole("searchbox", { name: "Find a source" }),
      "zzz"
    );

    expect(screen.getByText("No source matches")).toBeVisible();
  });
});
