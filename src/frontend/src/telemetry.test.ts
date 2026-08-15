/**
 * A recorded path names a screen, never a person: `/ic/<id>/personal` is one
 * screen whoever it is about.
 */
import { describe, expect, it } from "vitest";

import { screenPath } from "./telemetry";

describe("screenPath", () => {
  it("drops the person a page is about", () => {
    expect(
      screenPath("/ic/cccccccc-0000-0000-0000-000000000001/personal/git_output"),
    ).toBe("/ic/:id/personal/git_output");
  });

  it("leaves a path that names no one alone", () => {
    expect(screenPath("/portal/manage/platform-usage")).toBe(
      "/portal/manage/platform-usage",
    );
  });
});
