import { describe, expect, it } from "vitest";

import {
  dashboardSubject,
  documentTitle,
  titleScreenPath,
} from "./document-title";

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

  it("names home the way the root path is named", () => {
    expect(documentTitle("/portal")).toBe("Home · Insight");
    expect(documentTitle("/portal/overview")).toBe("Home · Insight");
    expect(documentTitle("/portal/overview/at-a-glance")).toBe(
      "Home · Insight"
    );
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
  it("composes a portal zone the way a recorded screen does", () => {
    expect(
      titleScreenPath("/portal", "?zone=manage&item=connector-health")
    ).toBe("/portal/manage/connector-health");
  });

  it("does not append a stale zone onto a custom or person path", () => {
    expect(
      titleScreenPath(
        "/portal/custom/delivery-overview",
        "?zone=overview&item=by-direction"
      )
    ).toBe("/portal/custom/delivery-overview");
    expect(
      titleScreenPath("/ic/ada/personal", "?zone=overview&item=git_output")
    ).toBe("/ic/ada/personal/git_output");
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
