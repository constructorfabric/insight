import { describe, expect, it } from "vitest";

import {
  dashboardSubject,
  documentTitle,
  titleScreenPath,
} from "./document-title";

describe("documentTitle", () => {
  it("puts the screen name's specific part first and Insight last", () => {
    expect(documentTitle("/home/directions")).toBe(
      "By direction · Overview · Insight"
    );
    expect(documentTitle("/explore/dev/git-output")).toBe(
      "Git output · Development · Insight"
    );
    expect(documentTitle("/people/ada/team/metrics")).toBe(
      "Metrics view · People · Insight"
    );
  });

  it("names the root path Home", () => {
    expect(documentTitle("/")).toBe("Home · Insight");
  });

  it("uses the translated name of an item", () => {
    expect(documentTitle("/home/attention")).toBe(
      "Worth a look · Overview · Insight"
    );
  });

  it("drops an id the screen name only echoes", () => {
    expect(documentTitle("/dashboards/delivery-overview")).toBe(
      "Dashboards · Insight"
    );
    expect(documentTitle("/dashboards/metrics/deploy-frequency")).toBe(
      "Metrics · Dashboards · Insight"
    );
    expect(documentTitle("/dashboards/alerts/rule-1/edit")).toBe(
      "Edit · Alerts · Dashboards · Insight"
    );
  });

  it("puts a known subject ahead of the screen name", () => {
    expect(
      documentTitle("/dashboards/delivery-overview", "Delivery overview")
    ).toBe("Delivery overview · Dashboards · Insight");
    expect(documentTitle("/people/ada/team/metrics", "Ada, Bo")).toBe(
      "Ada, Bo · Metrics view · People · Insight"
    );
    expect(documentTitle("/people/ada", "Ada")).toBe(
      "Ada · Person · Insight"
    );
  });

  it("leaves the screen name alone until a subject is known", () => {
    expect(documentTitle("/dashboards/delivery-overview", null)).toBe(
      "Dashboards · Insight"
    );
    expect(documentTitle("/dashboards/delivery-overview", "  ")).toBe(
      "Dashboards · Insight"
    );
  });

  it("keeps a path it cannot name as Insight", () => {
    expect(documentTitle("/nope")).toBe("Insight");
  });
});

describe("titleScreenPath", () => {
  it.each([
    ["/home", "/"],
    ["/home/", "/"],
    ["/home/directions", "/home/directions"],
    ["/people/ada/git-output", "/people/ada/git-output"],
    ["/dashboards/delivery-overview/", "/dashboards/delivery-overview"],
  ])("names %s as the screen %s", (pathname, expected) => {
    expect(titleScreenPath(pathname)).toBe(expected);
  });

  it("titles Home's default view as Home", () => {
    expect(documentTitle(titleScreenPath("/home"))).toBe("Home · Insight");
  });
});

describe("dashboardSubject", () => {
  it("prefers the board title and falls back to the stored name", () => {
    expect(dashboardSubject("Delivery overview", "delivery-overview")).toBe(
      "Delivery overview"
    );
    expect(dashboardSubject("  ", "delivery-overview")).toBe(
      "delivery-overview"
    );
    expect(dashboardSubject("", "")).toBeNull();
  });
});
