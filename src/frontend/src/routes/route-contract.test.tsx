// @vitest-environment jsdom
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock, portalRouter, matchesFor } = await import("@/test/portal-router");
  return {
    ...portalRouterMock(),
    redirect: (options: unknown) => ({ redirect: options }),
    createFileRoute: () => (options: Record<string, unknown>) => ({
      ...options,
      useParams: () => matchesFor(portalRouter.pathname)[0]?.params ?? {},
    }),
  };
});
vi.mock("@/components/portal/overview-view", () => ({
  OverviewView: (props: object) => <output>{JSON.stringify(["overview", props])}</output>,
}));
vi.mock("@/components/portal/direction-view", () => ({
  DirectionView: (props: object) => <output>{JSON.stringify(["directions", props])}</output>,
}));
vi.mock("@/components/portal/ai-cost-view", () => ({
  AiCostView: (props: object) => <output>{JSON.stringify(["aicost", props])}</output>,
}));
vi.mock("@/components/portal/person-view", () => ({
  PersonView: (props: object) => <output>{JSON.stringify(["person", props])}</output>,
}));
vi.mock("@/components/portal/people-view", () => ({
  PeopleView: (props: object) => <output>{JSON.stringify(["people", props])}</output>,
}));
vi.mock("@/components/portal/reports-view", () => ({
  ReportsView: (props: object) => <output>{JSON.stringify(["reports", props])}</output>,
}));
vi.mock("@/components/portal/manage-view", () => ({
  ManageView: (props: object) => <output>{JSON.stringify(["manage", props])}</output>,
}));

import { act, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { portalRouter } from "@/test/portal-router";

const sources = import.meta.glob("./*.tsx", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const routeSources = Object.entries(sources).filter(
  ([path]) => !path.includes(".test.") && !path.endsWith("__root.tsx"),
);

const routes = import.meta.glob(
  ["./_shell.{home,explore,people,reports,manage}.*.tsx", "!./*.test.tsx"],
  { import: "Route", eager: true },
) as Record<
  string,
  { component?: () => React.ReactNode }
>;

describe("every route", () => {
  it.each(routeSources)("%s paints something or sends the reader on", (_path, source) => {
    expect(source.includes("component:") || source.includes("redirect(")).toBe(true);
  });
});

const PERSON = "aaaaaaaa-1111-4111-8111-111111111111";

describe.each([
  ["/home", "./_shell.home.index.tsx", ["overview", { item: "at-a-glance" }]],
  ["/home/attention", "./_shell.home.$view.tsx", ["overview", { item: "attention" }]],
  ["/home/directions", "./_shell.home.$view.tsx", ["overview", { item: "by-direction" }]],
  ["/explore", "./_shell.explore.index.tsx", ["directions", { dir: "", lens: "" }]],
  [
    "/explore/dev/git-output",
    "./_shell.explore.$dir.$lens.tsx",
    ["directions", { dir: "dev", lens: "Git output" }],
  ],
  ["/explore/ai", "./_shell.explore.ai.index.tsx", ["aicost", { item: "overview" }]],
  ["/explore/ai/seats", "./_shell.explore.ai.$view.tsx", ["aicost", { item: "idle-seats" }]],
  [`/people/${PERSON}`, "./_shell.people.$person.index.tsx", ["person", { person: PERSON, section: null }]],
  [
    `/people/${PERSON}/git-output`,
    "./_shell.people.$person.$section.tsx",
    ["person", { person: PERSON, section: "git_output" }],
  ],
  [`/people/${PERSON}/team`, "./_shell.people.$person.team.index.tsx", ["people", { person: PERSON, item: "roster" }]],
  [
    `/people/${PERSON}/team/roster`,
    "./_shell.people.$person.team.$view.tsx",
    ["people", { person: PERSON, item: "employees" }],
  ],
  [
    `/people/${PERSON}/team/metrics`,
    "./_shell.people.$person.team.$view.tsx",
    ["people", { person: PERSON, item: "metrics" }],
  ],
  ["/reports/builder", "./_shell.reports.$view.tsx", ["reports", { item: "report-builder" }]],
  ["/manage/assistant", "./_shell.manage.$surface.tsx", ["manage", { item: "ai-assistant" }]],
  ["/manage/identities", "./_shell.manage.identities.index.tsx", ["manage", { item: "identities" }]],
  [
    "/manage/identities/person",
    "./_shell.manage.identities.$mode.tsx",
    ["manage", { item: "identities", mode: "person" }],
  ],
  [
    "/manage/ingestion/bamboohr",
    "./_shell.manage.ingestion.$connector.tsx",
    ["manage", { item: "ingestion", connector: "bamboohr" }],
  ],
])("%s", (path, file, expected) => {
  it("renders the page its path names", () => {
    act(() => portalRouter.reset(path));
    const Page = routes[file]?.component;
    if (!Page) throw new Error(`${file} paints nothing`);

    render(<Page />);

    expect(JSON.parse(screen.getByRole("status").textContent ?? "null"), path).toEqual(expected);
  });
});
