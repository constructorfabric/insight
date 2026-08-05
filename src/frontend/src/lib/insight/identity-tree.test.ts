/**
 * Lookups and roster derivation over the identity tree.
 *
 * `findIdentityNode` keys on the canonical person id since the cutover.
 *
 * `flattenSubordinates` marks depth-1 reports `is_direct`;
 * `scopeRosterToDirectReports` narrows a roster to those entries when the
 * "Direct reports only" toggle is on, and passes `null` through so screens
 * keep their roster-not-loaded gate. `hasIndirectReports` tells screens
 * whether that toggle can change anything at all (#1756).
 */

import { describe, expect, it } from "vitest";

import type { IdentityPerson } from "@/types/insight";
import { isPersonId } from "@/lib/metrics/entity";
import {
  findIdentityNode,
  flattenSubordinates,
  hasIndirectReports,
  scopeRosterToDirectReports,
  type RosterEntry,
} from "./identity-tree";

// One UUID per persona, derived from the local part so a failure names the
// person. person_id is deliberately NOT the email: keying them the same would
// hide a lookup that still matches on the wrong field.
function personId(email: string): string {
  const tag = email.split("@")[0]!.padEnd(4, "0").slice(0, 4);
  return `019e2803-0000-7000-8000-0000${Buffer.from(tag).toString("hex")}`;
}

function person(
  email: string,
  subordinates: IdentityPerson[] = [],
): IdentityPerson {
  return {
    person_id: personId(email),
    email,
    display_name: email.split("@")[0]!,
    subordinates,
  } as IdentityPerson;
}

const pivot = person("alice@x.io", [
  person("bob@x.io", [person("carol@x.io"), person("dave@x.io")]),
  person("erin@x.io"),
]);

describe("the persona fixture", () => {
  it("emits ids the route guard would accept — an invalid UUID here would let\
 these tests pass against ids production rejects", () => {
    expect(isPersonId(personId("alice@x.io"))).toBe(true);
  });
});

describe("findIdentityNode", () => {
  it("finds the root itself", () => {
    expect(findIdentityNode(pivot, personId("alice@x.io"))?.email).toBe(
      "alice@x.io",
    );
  });

  it("finds a transitive descendant", () => {
    expect(findIdentityNode(pivot, personId("carol@x.io"))?.email).toBe(
      "carol@x.io",
    );
  });

  it("matches regardless of UUID casing", () => {
    const upper = personId("dave@x.io").toUpperCase();
    expect(findIdentityNode(pivot, upper)?.email).toBe("dave@x.io");
  });

  it("returns null for someone outside the tree and for no tree at all", () => {
    expect(findIdentityNode(pivot, personId("zoe@x.io"))).toBeNull();
    expect(findIdentityNode(null, personId("alice@x.io"))).toBeNull();
    expect(findIdentityNode(undefined, personId("alice@x.io"))).toBeNull();
  });

  it("does not match a node by its email", () => {
    expect(findIdentityNode(pivot, "bob@x.io")).toBeNull();
  });
});

describe("flattenSubordinates", () => {
  it("marks only depth-1 reports as direct", () => {
    const roster = flattenSubordinates(pivot);
    expect(roster.map((r) => [r.email, r.is_direct])).toEqual([
      ["bob@x.io", true],
      ["carol@x.io", false],
      ["dave@x.io", false],
      ["erin@x.io", true],
    ]);
  });
});

describe("scopeRosterToDirectReports", () => {
  const roster: RosterEntry[] = flattenSubordinates(pivot);

  it("keeps only direct reports when scoping is on", () => {
    expect(
      scopeRosterToDirectReports(roster, true)?.map((r) => r.email),
    ).toEqual(["bob@x.io", "erin@x.io"]);
  });

  it("returns the roster unchanged when scoping is off", () => {
    expect(scopeRosterToDirectReports(roster, false)).toBe(roster);
  });

  it("passes null through regardless of the toggle", () => {
    expect(scopeRosterToDirectReports(null, true)).toBeNull();
    expect(scopeRosterToDirectReports(null, false)).toBeNull();
  });
});

describe("hasIndirectReports", () => {
  it("is true when the roster has at least one indirect report", () => {
    expect(hasIndirectReports(flattenSubordinates(pivot))).toBe(true);
  });

  it("is false when every report is direct (no subteams)", () => {
    const flat = flattenSubordinates(
      person("dave@x.io", [person("fay@x.io"), person("gil@x.io")]),
    );
    expect(hasIndirectReports(flat)).toBe(false);
  });

  it("is false for an empty or missing roster", () => {
    expect(hasIndirectReports([])).toBe(false);
    expect(hasIndirectReports(null)).toBe(false);
  });
});
