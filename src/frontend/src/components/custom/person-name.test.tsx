const name = vi.hoisted(() => ({
  state: { isPending: false, isError: false, data: null as string | null },
}));
vi.mock("@/queries/person-name", () => ({
  usePersonName: () => name.state,
}));

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { OwnerName } from "./person-name";

const PERSON = "01900000-0000-7000-8000-00000000000a";

describe("<OwnerName>", () => {
  it.each([
    ["a name that was read", { isPending: false, isError: false, data: "Jane Doe" }, "Owner: Jane Doe"],
    ["a person the roster names nothing", { isPending: false, isError: false, data: null }, null],
    ["a person the tenant search falls back to the id for", { isPending: false, isError: false, data: PERSON }, `Owner: ${PERSON}`],
    ["a person the caller may not see", { isPending: false, isError: true, data: null }, null],
    ["a name still being read", { isPending: true, isError: false, data: null }, null],
  ])("shows %s as %s", (_case, state, shown) => {
    name.state = state;
    const { container } = render(<OwnerName personId={PERSON} />);

    if (shown === null) {
      expect(container, `case: ${_case}`).toBeEmptyDOMElement();
    } else {
      expect(screen.getByText(shown)).toBeInTheDocument();
    }
  });
});
