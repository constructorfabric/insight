/**
 * Two colleagues can carry the same display name, so the handle disambiguates.
 */
import { describe, expect, it } from "vitest";

import { visitorLabel } from "./visitor-label";

const PERSON_ID = "4d1f0a6c-0000-4000-8000-0000000000aa";

describe("visitorLabel", () => {
  it("shows the name and keeps the handle for the hover", () => {
    expect(
      visitorLabel({
        person_id: PERSON_ID,
        display_name: "Ada Lovelace",
        username: "ada",
      })
    ).toEqual({ label: "Ada Lovelace", detail: "username: ada" });
  });

  it("falls back to the handle when no name was mirrored", () => {
    expect(
      visitorLabel({ person_id: PERSON_ID, display_name: "", username: "ada" })
    ).toEqual({ label: "ada", detail: "username: ada" });
  });

  it("falls back to the id when neither a name nor a handle is known", () => {
    expect(
      visitorLabel({ person_id: PERSON_ID, display_name: "", username: "" })
    ).toEqual({
      label: PERSON_ID,
      detail: PERSON_ID,
    });
  });

  it("still names a row served by an API that sends no handle at all", () => {
    expect(
      visitorLabel({ person_id: PERSON_ID, display_name: "Ada Lovelace" })
    ).toEqual({
      label: "Ada Lovelace",
      detail: PERSON_ID,
    });
  });

  it("ignores whitespace-only values from the identity rows", () => {
    expect(
      visitorLabel({
        person_id: PERSON_ID,
        display_name: "  ",
        username: " ada ",
      })
    ).toEqual({ label: "ada", detail: "username: ada" });
  });
});
