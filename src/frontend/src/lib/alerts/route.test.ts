import { describe, expect, it } from "vitest";

import { alertIdFromPath } from "./route";

describe("alertIdFromPath", () => {
  it.each([
    ["/dashboards/alerts/a1", "a1"],
    ["/dashboards/alerts/a1/edit", "a1"],
    ["/dashboards/alerts/new", ""],
    ["/dashboards/alerts", ""],
    ["/dashboards/metrics/a1", ""],
  ])("reads %s as %o", (path, id) => {
    expect(alertIdFromPath(path), `path: ${path}`).toBe(id);
  });

  it.each([
    ["/dashboards/alerts/%E0%A4%A", "%E0%A4%A"],
    ["/dashboards/alerts/a%25b", "a%25b"],
    ["/dashboards/alerts/a%2Fb/edit", "a%2Fb"],
  ])("hands back the segment as the router left it: %s", (path, id) => {
    expect(() => alertIdFromPath(path), `path: ${path}`).not.toThrow();
    expect(alertIdFromPath(path), `path: ${path}`).toBe(id);
  });
});
