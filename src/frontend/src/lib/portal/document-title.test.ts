import { describe, expect, it } from "vitest";

import { defaultZoneItem, resolveZoneItem } from "@/lib/portal/nav-model";

import {
  dashboardSubject,
  documentTitle,
  titleScreenPath,
} from "./document-title";

function screen(over: Partial<Parameters<typeof titleScreenPath>[0]> = {}) {
  return titleScreenPath({
    pathname: "/portal",
    search: "",
    zone: "overview",
    item: null,
    ...over,
  });
}

describe("documentTitle", () => {
  it("puts the screen name's specific part first and Insight last", () => {
    expect(documentTitle("/portal/overview/by-direction")).toBe(
      "By direction · Overview · Insight"
    );
    expect(documentTitle("/portal/directions/dev/git-output")).toBe(
      "Git output · Development · Directions · Insight"
    );
    expect(documentTitle("/ic/ada/team/metrics")).toBe(
      "Metrics view · People · Insight"
    );
  });

  it("names the root path Home", () => {
    expect(documentTitle("/")).toBe("Home · Insight");
  });

  it("uses the translated name of an item", () => {
    expect(documentTitle("/portal/overview/attention")).toBe(
      "Worth a look · Overview · Insight"
    );
  });

  it("drops an id the screen name only echoes", () => {
    expect(documentTitle("/portal/custom/delivery-overview")).toBe(
      "Dashboards · Insight"
    );
    expect(documentTitle("/portal/custom/metrics/deploy-frequency")).toBe(
      "Metrics · Dashboards · Insight"
    );
    expect(documentTitle("/portal/custom/alerts/rule-1/edit")).toBe(
      "Edit · Alerts · Dashboards · Insight"
    );
  });

  it("puts a known subject ahead of the screen name", () => {
    expect(
      documentTitle("/portal/custom/delivery-overview", "Delivery overview")
    ).toBe("Delivery overview · Dashboards · Insight");
    expect(documentTitle("/ic/ada/team/metrics", "Ada, Bo")).toBe(
      "Ada, Bo · Metrics view · People · Insight"
    );
    expect(documentTitle("/ic/ada/personal", "Ada")).toBe(
      "Ada · Personal · Person · Insight"
    );
  });

  it("leaves the screen name alone until a subject is known", () => {
    expect(documentTitle("/portal/custom/delivery-overview", null)).toBe(
      "Dashboards · Insight"
    );
    expect(documentTitle("/portal/custom/delivery-overview", "  ")).toBe(
      "Dashboards · Insight"
    );
  });

  it("keeps a path it cannot name as Insight", () => {
    expect(documentTitle("/nope")).toBe("Insight");
  });
});

describe("titleScreenPath", () => {
  it("names home only when the resolved zone is the overview default", () => {
    const home = defaultZoneItem("overview");
    expect(screen({ item: home })).toBe("/");
    expect(documentTitle(screen({ item: home }))).toBe("Home · Insight");
    expect(screen({ item: "by-direction" })).toBe("/portal/overview/by-direction");
  });

  it("names a bare portal as the person page when that is the resolved zone", () => {
    const path = screen({ zone: "person", item: null });
    expect(path).toBe("/ic/viewer/personal");
    expect(documentTitle(path, "Ada")).toBe("Ada · Personal · Person · Insight");
  });

  it("uses the people item the shell resolves, not a stray section id", () => {
    const item = resolveZoneItem("people", "git_output");
    const path = screen({
      pathname: "/ic/bob/team",
      search: "?item=git_output",
      zone: "people",
      item,
    });
    expect(path).toBe("/ic/bob/team/roster");
    expect(documentTitle(path)).toBe("My team · People · Insight");
  });

  it("keeps a person section the person page actually opens", () => {
    expect(
      screen({
        pathname: "/ic/ada/personal",
        search: "?item=git_output",
        zone: "person",
        item: "git_output",
      })
    ).toBe("/ic/ada/personal/git_output");
  });

  it("composes a portal zone the way a recorded screen does", () => {
    expect(
      screen({
        search: "?zone=manage&item=connector-health",
        zone: "manage",
        item: "connector-health",
      })
    ).toBe("/portal/manage/connector-health");
  });

  it("does not append a stale zone onto a custom path", () => {
    expect(
      screen({
        pathname: "/portal/custom/delivery-overview",
        search: "?zone=overview&item=by-direction",
        zone: "custom",
        item: "by-direction",
      })
    ).toBe("/portal/custom/delivery-overview");
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
