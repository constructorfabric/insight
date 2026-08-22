/**
 * One precedence for every person label: display name → username → e-mail →
 * person id (#2711). A blank or whitespace-only value does not count as
 * present, so a person with an empty display name is still named by their
 * handle instead of rendering a blank row.
 */
import { describe, expect, it } from "vitest";

import {
  personDisplayName,
  personName,
  type PersonNaming,
} from "./person-display";

describe("person naming precedence", () => {
  it.each<[string, PersonNaming, string]>([
    [
      "display name wins over every other field",
      { display_name: "Ann Smith", username: "asmith", email: "a@example.com" },
      "Ann Smith",
    ],
    [
      "an empty display name falls back to the username",
      { display_name: "", username: "asmith", email: "a@example.com" },
      "asmith",
    ],
    [
      "a whitespace-only display name is not a name",
      { display_name: "   ", username: "asmith" },
      "asmith",
    ],
    [
      "the e-mail comes only after the username",
      { display_name: "", username: "", email: "1+a@noreply.example.com" },
      "1+a@noreply.example.com",
    ],
    [
      "absent fields read the same as empty ones",
      { username: "asmith" },
      "asmith",
    ],
    [
      "null fields read the same as empty ones",
      { display_name: null, username: null, email: "a@example.com" },
      "a@example.com",
    ],
  ])("%s", (_rule, fields, expected) => {
    expect(personName(fields)).toBe(expected);
  });

  it("knows no name when every field is blank", () => {
    expect(personName({ display_name: "", username: " ", email: "" })).toBeNull();
  });

  it("falls back to the person id only after every named field", () => {
    const anonymous = { person_id: "p-anon", display_name: "", username: "" };
    expect(personDisplayName(anonymous)).toBe("p-anon");
  });
});
