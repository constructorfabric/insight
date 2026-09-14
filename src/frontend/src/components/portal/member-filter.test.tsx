import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { MemberFilter } from "./member-filter";
import type { MemberSelection } from "./member-selection";

const MEMBERS = [
  { entityId: "manager", displayName: "Manager" },
  { entityId: "zara", displayName: "Zara" },
  { entityId: "alex", displayName: "Alex" },
];

describe("MemberFilter", () => {
  it("searches the scoped options without changing the selection", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn<(selection: MemberSelection) => void>();
    render(
      <MemberFilter
        members={MEMBERS}
        selection={{ kind: "all" }}
        onChange={onChange}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: "Filter people, all 3 shown" }),
    );
    await user.type(screen.getByRole("searchbox", { name: "Find someone" }), "za");

    expect(screen.getByRole("checkbox", { name: "Zara" })).toBeChecked();
    expect(screen.queryByRole("checkbox", { name: "Manager" })).toBeNull();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("opens and applies actions from the keyboard", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn<(selection: MemberSelection) => void>();
    render(
      <MemberFilter
        members={MEMBERS}
        selection={{ kind: "all" }}
        onChange={onChange}
      />,
    );

    const trigger = screen.getByRole("button", {
      name: "Filter people, all 3 shown",
    });
    trigger.focus();
    await user.keyboard("{Enter}");

    const none = screen.getByRole("button", { name: "None" });
    none.focus();
    await user.keyboard("{Enter}");

    expect(onChange).toHaveBeenCalledWith({
      kind: "selected",
      ids: new Set<string>(),
    });
  });

  it("restores all scoped people", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn<(selection: MemberSelection) => void>();
    render(
      <MemberFilter
        members={MEMBERS}
        selection={{ kind: "selected", ids: new Set(["zara"]) }}
        onChange={onChange}
      />,
    );

    await user.click(
      screen.getByRole("button", { name: "Filter people, 1 of 3 shown" }),
    );
    await user.click(screen.getByRole("button", { name: "All" }));

    expect(onChange).toHaveBeenCalledWith({ kind: "all" });
  });
});
