/**
 * A recorded path names a screen; the report has to name it the way the
 * product does. Two screens that differ must get titles that differ — a table
 * where `/ic/:id/personal` and `/ic/:id/team` read alike is worse than raw
 * paths.
 */
import { describe, expect, it } from "vitest";

import { screenLabel } from "./screen-label";

describe("screenLabel", () => {
  it.each([
    ["/home", "Overview"],
    ["/home/directions", "Overview › By direction"],
    ["/explore", "Directions"],
    ["/explore/dev/git-output", "Development › Git output"],
    ["/explore/ai", "AI & Cost"],
    ["/explore/ai/seats", "AI & Cost › Idle seats"],
    ["/people/:id", "Person"],
    ["/people/:id/git-output", "Person › Git output"],
    ["/people/:id/team", "People"],
    ["/people/:id/team/roster", "People › Roster · by role"],
    ["/reports/builder", "Reports › Report builder"],
    ["/manage/usage", "Manage › Platform usage"],
    ["/manage/identities/person", "Manage › Identities · roles & taxonomy › A person and their accounts"],
    ["/manage/ingestion/bamboohr", "Manage › Ingestion › bamboohr"],
    ["/dashboards", "Dashboards"],
  ])("names %s as %s", (path, label) => {
    expect(screenLabel(path), path).toBe(label);
  });

  it("names a zone and the item inside it", () => {
    expect(screenLabel("/portal/manage/platform-usage")).toBe(
      "Manage › Platform usage",
    );
    expect(screenLabel("/portal/overview/by-direction")).toBe("Overview › By direction");
    expect(screenLabel("/portal/people")).toBe("People");
  });

  it("tells the two person views apart", () => {
    expect(screenLabel("/ic/:id/personal")).toBe("Person › Personal");
    expect(screenLabel("/ic/:id/personal/git_output")).toBe(
      "Person › Personal › Git output",
    );
  });

  it("names the team route by the zone it opens — People, not Person", () => {
    expect(screenLabel("/ic/:id/team")).toBe("People");
    expect(screenLabel("/ic/:id/team/roster")).toBe("People › My team");
    expect(screenLabel("/ic/:id/team/employees")).toBe("People › Roster · by role");
  });

  it("names a metric group reached on the team route", () => {
    expect(screenLabel("/ic/:id/team/git_output")).toBe("People › Git output");
  });

  it("names a section reached without a zone in the url", () => {
    expect(screenLabel("/portal/collaboration")).toBe("Collaboration");
    expect(screenLabel("/portal/git_output")).toBe("Git output");
    expect(screenLabel("/portal/ai_adoption")).toBe("AI adoption");
    expect(screenLabel("/portal/wiki")).toBe("Wiki");
    expect(screenLabel("/portal/task_delivery")).toBe("Task delivery");
  });

  it("names a direction and the lens inside it", () => {
    expect(screenLabel("/portal/directions/dev")).toBe("Directions › Development");
    expect(screenLabel("/portal/directions/dev/git-output")).toBe(
      "Directions › Development › Git output",
    );
    expect(screenLabel("/portal/directions/collab/files-sharing")).toBe(
      "Directions › Collaboration › Files & sharing",
    );
  });

  it("names the mode the identities console was opened in", () => {
    expect(screenLabel("/portal/manage/identities/queue")).toBe(
      "Manage › Identities · roles & taxonomy › Review queue",
    );
    expect(screenLabel("/portal/manage/identities/person")).toBe(
      "Manage › Identities · roles & taxonomy › A person and their accounts",
    );
    expect(screenLabel("/portal/manage/identities/accounts")).toBe(
      "Manage › Identities · roles & taxonomy › An account and whose it is",
    );
  });

  it("keeps a direction, lens or mode it cannot name rather than inventing one", () => {
    expect(screenLabel("/portal/directions/nope")).toBe("Directions › nope");
    expect(screenLabel("/portal/directions/dev/nope")).toBe(
      "Directions › Development › nope",
    );
    expect(screenLabel("/portal/manage/identities/nope")).toBe(
      "Manage › Identities · roles & taxonomy › nope",
    );
  });

  it.each([
    ["/dashboards/:id", "Dashboards › Dashboard"],
    ["/dashboards/q3-hiring-plan", "Dashboards › q3-hiring-plan"],
    ["/dashboards/metrics/deploy-frequency", "Dashboards › Metrics › deploy-frequency"],
    ["/dashboards/metrics/:id", "Dashboards › Metrics › Metric"],
    [
      "/dashboards/edit/dashboards/release-health",
      "Dashboards › Edit › Dashboards › release-health",
    ],
    ["/dashboards/edit/dashboards/:id", "Dashboards › Edit › Dashboards › Dashboard"],
    ["/dashboards/alerts/:id/edit", "Dashboards › Alerts › Alert › Edit"],
  ])("names the custom page %s as %s", (path, title) => {
    expect(screenLabel(path), `path: ${path}`).toBe(title);
  });

  it("gives every custom page its own title", () => {
    const paths = [
      "/dashboards/:id",
      "/dashboards/q3-hiring-plan",
      "/dashboards/release-health",
      "/dashboards/metrics",
      "/dashboards/metrics/deploy-frequency",
      "/dashboards/metrics/:id",
      "/dashboards/new/dashboards",
      "/dashboards/new/metrics",
      "/dashboards/edit/dashboards/:id",
      "/dashboards/edit/dashboards/release-health",
      "/dashboards/edit/metrics/deploy-frequency",
      "/dashboards/alerts/new",
      "/dashboards/alerts/:id",
      "/dashboards/alerts/:id/edit",
    ];

    const titles = paths.map(screenLabel);
    const shared = paths.filter(
      (_, i) => titles.indexOf(titles[i]) !== titles.lastIndexOf(titles[i]),
    );

    expect(shared, "paths that share a title").toEqual([]);
  });

  it("keeps a path it cannot name rather than inventing one", () => {
    expect(screenLabel("/some/new/route")).toBe("/some/new/route");
    expect(screenLabel("/portal/manage/not-a-real-item")).toBe(
      "Manage › not-a-real-item",
    );
  });

  it("names the two roots", () => {
    expect(screenLabel("/portal")).toBe("Portal");
    expect(screenLabel("/")).toBe("Home");
  });
});
