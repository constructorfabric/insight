import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

let pathname = "/ic/alice/personal";

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    children,
    ...rest
  }: {
    to?: string;
    children?: React.ReactNode;
  } & Record<string, unknown>) => (
    <a data-to={to} {...(rest as object)}>
      {children}
    </a>
  ),
  useRouterState: ({
    select,
  }: {
    select: (s: { location: { pathname: string } }) => string;
  }) => select({ location: { pathname } }),
}));

import { IcViewToggle } from "./ic-view-toggle";

describe("IcViewToggle", () => {
  beforeEach(() => {
    pathname = "/ic/alice/personal";
  });

  it("renders nothing for people without reports", () => {
    const { container } = render(
      <IcViewToggle person="alice" hasReports={false} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("marks the personal view active on the personal route", () => {
    render(<IcViewToggle person="alice" hasReports />);

    const personal = screen.getByText("Personal").closest("a");
    const team = screen.getByText("Team").closest("a");
    expect(personal).toHaveAttribute("aria-pressed", "true");
    expect(team).toHaveAttribute("aria-pressed", "false");
  });

  it("marks the team view active on the team route", () => {
    pathname = "/ic/alice/team";
    render(<IcViewToggle person="alice" hasReports />);

    expect(screen.getByText("Team").closest("a")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });
});
