import { describe, expect, it } from "vitest";

import {
  compareDocumentTitle,
  dashboardDisplayName,
  dashboardDocumentTitle,
  formatDocumentTitle,
  openDashboardTitle,
  pageDocumentTitle,
  personDocumentTitle,
  teamDocumentTitle,
  type PageTitleInput,
} from "./document-title";

function route(over: Partial<PageTitleInput> = {}): PageTitleInput {
  return {
    pathname: "/portal",
    zone: "overview",
    item: null,
    dir: "",
    lens: "",
    peopleFlat: false,
    ...over,
  };
}

describe("formatDocumentTitle", () => {
  it("puts the specific part first and the app last", () => {
    expect(formatDocumentTitle(["Metrics view", "People"])).toBe(
      "Metrics view · People · Insight"
    );
  });

  it("drops blank parts", () => {
    expect(formatDocumentTitle(["  Ada  ", "", "   "])).toBe("Ada · Insight");
    expect(formatDocumentTitle(["", "   "])).toBe("Insight");
    expect(formatDocumentTitle([])).toBe("Insight");
  });
});

describe("pageDocumentTitle", () => {
  it("names home", () => {
    expect(pageDocumentTitle(route())).toBe("Home · Insight");
    expect(pageDocumentTitle(route({ zone: null, pathname: "/portal" }))).toBe(
      "Home · Insight"
    );
    expect(pageDocumentTitle(route({ item: "at-a-glance" }))).toBe(
      "Home · Insight"
    );
  });

  it("names an overview item by its own label", () => {
    expect(pageDocumentTitle(route({ item: "by-direction" }))).toBe(
      "By direction · Home · Insight"
    );
    expect(pageDocumentTitle(route({ item: "attention" }))).toBe(
      "Worth a look · Home · Insight"
    );
  });

  it("names a direction and the lens inside it", () => {
    expect(
      pageDocumentTitle(
        route({ zone: "directions", dir: "dev", lens: "Git output" })
      )
    ).toBe("Git output · Development · Directions · Insight");
    expect(
      pageDocumentTitle(
        route({ zone: "directions", dir: "dev", lens: "git-output" })
      )
    ).toBe("Git output · Development · Directions · Insight");
    expect(pageDocumentTitle(route({ zone: "directions", dir: "dev" }))).toBe(
      "Development · Directions · Insight"
    );
  });

  it("keeps a direction or lens it cannot name", () => {
    expect(pageDocumentTitle(route({ zone: "directions" }))).toBe(
      "Directions · Insight"
    );
    expect(
      pageDocumentTitle(route({ zone: "directions", dir: "dev", lens: "nope" }))
    ).toBe("nope · Development · Directions · Insight");
  });

  it("names the people views", () => {
    expect(pageDocumentTitle(route({ zone: "people" }))).toBe(
      "My team · People · Insight"
    );
    expect(pageDocumentTitle(route({ zone: "people", item: "metrics" }))).toBe(
      "Metrics view · People · Insight"
    );
    expect(
      pageDocumentTitle(route({ zone: "people", item: "employees" }))
    ).toBe("Roster · by role · People · Insight");
  });

  it("names a flat organisation's people views", () => {
    expect(pageDocumentTitle(route({ zone: "people", peopleFlat: true }))).toBe(
      "Overview · People · Insight"
    );
    expect(
      pageDocumentTitle(
        route({ zone: "people", item: "employees", peopleFlat: true })
      )
    ).toBe("Roster · People · Insight");
  });

  it.each([
    ["/ic/ada/personal", "git_output", "Git output · Person · Insight"],
    ["/ic/ada/team", "metrics", "Metrics view · People · Insight"],
  ])("names %s from the path when no zone is set", (pathname, item, title) => {
    expect(
      pageDocumentTitle(route({ pathname, zone: null, item })),
      pathname
    ).toBe(title);
  });

  it("names the manage page that is open", () => {
    expect(
      pageDocumentTitle(route({ zone: "manage", item: "connector-health" })),
      "connector health"
    ).toBe("Sources & connectors · Manage · Insight");
  });

  it("names the identities mode that is open", () => {
    expect(
      pageDocumentTitle(
        route({ zone: "manage", item: "identities", mode: "queue" })
      ),
      "review queue"
    ).toBe("Review queue · Identities · roles & taxonomy · Manage · Insight");
  });

  it.each([
    [{ zone: "aicost" }, "AI & Cost · Insight"],
    [
      { zone: "aicost", item: "spend-by-tool" },
      "Spend by tool · AI & Cost · Insight",
    ],
    [{ zone: "reports" }, "Report builder · Reports · Insight"],
  ] as const)("names %j", (over, title) => {
    expect(pageDocumentTitle(route(over)), title).toBe(title);
  });

  it.each([
    ["/portal/custom/", "Dashboards · Insight"],
    ["/portal/custom/metrics", "Metrics · Dashboards · Insight"],
    ["/portal/custom/widgets/deploys", "Widgets · Dashboards · Insight"],
    ["/portal/custom/alerts/new", "New alert · Alerts · Dashboards · Insight"],
    [
      "/portal/custom/alerts/rule-1/edit",
      "Edit alert · Alerts · Dashboards · Insight",
    ],
    ["/portal/custom/new/metrics", "New metric · Dashboards · Insight"],
    [
      "/portal/custom/edit/dashboards/release-health",
      "Edit dashboard · Dashboards · Insight",
    ],
  ])("names %s as %s", (pathname, title) => {
    expect(pageDocumentTitle(route({ pathname })), pathname).toBe(title);
  });

  it("does not use a dashboard url slug as its name", () => {
    expect(
      pageDocumentTitle(
        route({ pathname: "/portal/custom/delivery-overview", zone: "custom" })
      )
    ).toBe("Dashboards · Insight");
  });

  it("keeps a path it cannot name as the app title", () => {
    expect(pageDocumentTitle(route({ pathname: "/nope", zone: null }))).toBe(
      "Insight"
    );
  });
});

describe("dashboardDocumentTitle", () => {
  it("prefers the board title and falls back to the stored name", () => {
    expect(dashboardDisplayName("Delivery overview", "delivery-overview")).toBe(
      "Delivery overview"
    );
    expect(dashboardDisplayName("  ", "delivery-overview")).toBe(
      "delivery-overview"
    );
    expect(dashboardDisplayName("", "")).toBeNull();
    expect(
      dashboardDocumentTitle(dashboardDisplayName("Release health", "old"))
    ).toBe("Release health · Dashboards · Insight");
  });

  it("returns null while the name is missing so the route title stands", () => {
    expect(dashboardDocumentTitle(null)).toBeNull();
    expect(dashboardDocumentTitle("   ")).toBeNull();
    expect(openDashboardTitle(null)).toBeNull();
  });

  it("uses the loaded board's title once a read exists", () => {
    expect(openDashboardTitle({ title: "Release health", name: "old" })).toBe(
      "Release health · Dashboards · Insight"
    );
    expect(openDashboardTitle({ title: "  ", name: "old-name" })).toBe(
      "old-name · Dashboards · Insight"
    );
  });
});

describe("compareDocumentTitle", () => {
  it("names the people being compared, in the given order", () => {
    expect(compareDocumentTitle(["Ada", "Bo"])).toBe(
      "Ada, Bo · Metrics view · People · Insight"
    );
  });

  it("returns null until a compared person has a name", () => {
    expect(compareDocumentTitle([])).toBeNull();
    expect(compareDocumentTitle(["  "])).toBeNull();
  });
});

describe("personDocumentTitle", () => {
  it("names the person, and the section when one is open", () => {
    expect(personDocumentTitle("Ada", null)).toBe("Ada · Person · Insight");
    expect(personDocumentTitle("Ada", "Git output")).toBe(
      "Git output · Ada · Person · Insight"
    );
  });
});

describe("teamDocumentTitle", () => {
  it("names whose team is on screen", () => {
    expect(teamDocumentTitle("Ada")).toBe("Ada's team · People · Insight");
  });

  it("returns null until the team has a name", () => {
    expect(teamDocumentTitle("  ")).toBeNull();
  });
});
