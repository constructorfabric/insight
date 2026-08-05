import { render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@tanstack/react-router")>();
  return {
    ...actual,
    Link: ({
      to,
      params,
      children,
    }: {
      to?: string;
      params?: Record<string, string>;
      children?: React.ReactNode;
    }) => (
      <a href={(to ?? "").replace("$person", params?.person ?? "")}>
        {children}
      </a>
    ),
  };
});

vi.mock("@/hooks/use-settings", () => ({
  useSettings: () => ({ focusMode: "all" }),
}));

import { TriageList } from "@/components/widgets/dashboard/triage-list";
import type { TeamMember } from "@/types/insight";

function member(person_id: string, name: string): TeamMember {
  return { person_id, name } as TeamMember;
}

describe("TriageList", () => {
  it("labels each member behind / ahead / on par and links to their IC view", () => {
    render(
      <TriageList
        rows={[
          {
            member: member("a@x.com", "Ann"),
            belowCount: 2,
            topCount: 0,
            worstMetricLabel: "Resolution",
          },
          {
            member: member("b@x.com", "Bo"),
            belowCount: 0,
            topCount: 3,
            worstMetricLabel: null,
          },
          {
            member: member("c@x.com", "Cy"),
            belowCount: 0,
            topCount: 0,
            worstMetricLabel: null,
          },
        ]}
      />,
    );

    const ann = screen.getByText("Ann").closest("a")!;
    expect(ann).toHaveAttribute("href", "/ic/a@x.com/personal");
    expect(within(ann).getByText("2 behind peers")).toBeInTheDocument();
    expect(within(ann).getByText("worst: Resolution")).toBeInTheDocument();

    expect(
      within(screen.getByText("Bo").closest("a")!).getByText("3 ahead of peers"),
    ).toBeInTheDocument();
    expect(
      within(screen.getByText("Cy").closest("a")!).getByText(
        "on par with peers",
      ),
    ).toBeInTheDocument();
  });
});
