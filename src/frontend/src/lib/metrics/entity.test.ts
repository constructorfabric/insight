import { describe, expect, it } from "vitest";

import { isPersonId, normalizePersonId, personIdFromPath } from "./entity";

describe("normalizePersonId", () => {
  it("trims and lowercases so a route param and an identity record compare equal", () => {
    expect(normalizePersonId("  019E27BC-DEC0-7626-81A9-C5524662A6A9 ")).toBe(
      "019e27bc-dec0-7626-81a9-c5524662a6a9",
    );
  });
});

describe("isPersonId", () => {
  it("accepts a canonical person UUID in any casing", () => {
    expect(isPersonId("019e27bc-dec0-7626-81a9-c5524662a6a9")).toBe(true);
    expect(isPersonId(" 019E27BC-DEC0-7626-81A9-C5524662A6A9 ")).toBe(true);
  });

  it("rejects the pre-cutover email key so the route can redirect instead of 400", () => {
    expect(isPersonId("alice@example.com")).toBe(false);
  });

  it("rejects the nil UUID, which parses but is never a person", () => {
    // Both backends reject it. Accepting it here would clear the route guard
    // and paint a dashboard whose every metric request 400s.
    expect(isPersonId("00000000-0000-0000-0000-000000000000")).toBe(false);
    expect(isPersonId(" 00000000-0000-0000-0000-000000000000 ")).toBe(false);
  });

  it.each(["", "   ", "019e27bc", "019e27bc-dec0-7626-81a9-c5524662a6a9-extra"])(
    "rejects malformed value %j",
    (value) => {
      expect(isPersonId(value)).toBe(false);
    },
  );
});

describe("personIdFromPath", () => {
  const ID = "019e27bc-dec0-7626-81a9-c5524662a6a9";

  it("reads the person id a /ic/ path names, encoded or not", () => {
    expect(personIdFromPath(`/ic/${ID}/personal`)).toBe(ID);
    expect(personIdFromPath(`/ic/${ID}/team/`)).toBe(ID);
    expect(personIdFromPath("/ic/who%40x/personal")).toBe("who@x");
  });

  it("is null for a path that names no person", () => {
    expect(personIdFromPath("/")).toBeNull();
    expect(personIdFromPath("/metrics")).toBeNull();
  });

  it("survives a malformed percent-sequence instead of throwing", () => {
    // The pathname is reader-editable text; decoding it raw during render
    // raises URIError and takes the shell down over a typo'd link.
    expect(personIdFromPath("/ic/%E0%A4%A/personal")).toBeNull();
  });
});
