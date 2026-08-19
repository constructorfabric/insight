/**
 * A recorded path names a screen; the report has to name it the way the
 * product does. Two screens that differ must get titles that differ — a table
 * where `/ic/:id/personal` and `/ic/:id/team` read alike is worse than raw
 * paths.
 */
import { describe, expect, it } from "vitest";

import { screenLabel } from "./screen-label";

describe("screenLabel", () => {
  it("names a zone and the item inside it", () => {
    expect(screenLabel("/portal/manage/platform-usage")).toBe(
      "Manage › Platform usage",
    );
    expect(screenLabel("/portal/overview/trend")).toBe("Overview › Trend");
    expect(screenLabel("/portal/people")).toBe("People");
  });

  it("tells the two person views apart", () => {
    expect(screenLabel("/ic/:id/personal")).toBe("Person › Personal");
    expect(screenLabel("/ic/:id/team")).toBe("Person › Team");
    expect(screenLabel("/ic/:id/personal/git_output")).toBe(
      "Person › Personal › Git output",
    );
    expect(screenLabel("/ic/:id/team/git_output")).toBe("Person › Team › Git output");
  });

  it("names a section reached without a zone in the url", () => {
    expect(screenLabel("/portal/collaboration")).toBe("Collaboration");
    expect(screenLabel("/portal/git_output")).toBe("Git output");
    expect(screenLabel("/portal/ai_adoption")).toBe("AI adoption");
    expect(screenLabel("/portal/wiki")).toBe("Wiki");
    expect(screenLabel("/portal/task_delivery")).toBe("Task delivery");
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
