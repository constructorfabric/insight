import { describe, expect, it } from "vitest";

import type { IdentityPerson } from "@/types/insight";
import { resolveScopeRoster } from "./use-org-scope";

// Ids deliberately look nothing like emails: the scope resolver keys on
// `person_id` since the identity cutover, and an email-shaped fixture would
// hide a regression that reads the wrong field.
const person = (
  personId: string,
  name: string,
  subordinates: IdentityPerson[] = [],
): IdentityPerson =>
  ({
    person_id: personId,
    email: `${name.toLowerCase().replace(/ /g, ".")}@x`,
    display_name: name,
    subordinates,
  }) as unknown as IdentityPerson;

//        ao
//   ┌────┴─────┐
//  lead1      ic3
//  ┌──┴──┐
// ic1   lead2
//        │
//       ic2
const TREE = person("p-ao", "Ao", [
  person("p-lead1", "Lead One", [
    person("p-ic1", "IC One"),
    person("p-lead2", "Lead Two", [person("p-ic2", "IC Two")]),
  ]),
  person("p-ic3", "IC Three"),
]);

describe("resolveScopeRoster", () => {
  it("defaults to the viewer's whole subtree", () => {
    const s = resolveScopeRoster(TREE, "p-ao", { root: null, directOnly: false });
    expect(s.label).toBe("Ao");
    expect(s.count).toBe(5);
  });
  it("scopes to a sub-lead's subtree", () => {
    const s = resolveScopeRoster(TREE, "p-ao", { root: "p-lead1", directOnly: false });
    expect(s.label).toBe("Lead One");
    expect(s.roster?.map((r) => r.person_id).sort()).toEqual([
      "p-ic1",
      "p-ic2",
      "p-lead2",
    ]);
  });
  it("narrows to direct reports", () => {
    const s = resolveScopeRoster(TREE, "p-ao", { root: "p-lead1", directOnly: true });
    expect(s.roster?.map((r) => r.person_id).sort()).toEqual(["p-ic1", "p-lead2"]);
  });
  it("falls back to the viewer when root is outside the tree", () => {
    const s = resolveScopeRoster(TREE, "p-ao", { root: "p-stranger", directOnly: false });
    expect(s.label).toBe("Ao");
  });
  it("does not resolve a root outside the VIEWER's own subtree", () => {
    // lead2 viewing with root=lead1: lead1 exists in the full tree but not in
    // lead2's own subtree — the permission boundary must win, falling back
    // to the viewer (lead2), not resolving to lead1.
    const s = resolveScopeRoster(TREE, "p-lead2", { root: "p-lead1", directOnly: false });
    expect(s.label).toBe("Lead Two");
  });
  it("directOnly is a no-op when the pivot has no indirect reports", () => {
    const s = resolveScopeRoster(TREE, "p-ao", { root: "p-lead2", directOnly: true });
    expect(s.roster?.map((r) => r.person_id)).toEqual(["p-ic2"]);
    expect(s.canDirectOnly).toBe(false);
  });
  it("lists manager nodes for the picker, depth-annotated", () => {
    const s = resolveScopeRoster(TREE, "p-ao", { root: null, directOnly: false });
    expect(s.managerNodes.map((m) => `${"·".repeat(m.depth)}${m.person_id}`)).toEqual([
      "p-ao",
      "·p-lead1",
      "··p-lead2",
    ]);
    expect(s.managerNodes.map((m) => m.teamSize)).toEqual([5, 3, 1]);
  });

  it("keeps the picker in outline order across branches, not by depth", () => {
    // Two leads at the same level, each with a lead under them: the picker is
    // read as an org chart, so a lead's own leads follow it rather than all
    // depth-1 nodes coming before all depth-2 nodes.
    const tree = person("p-top", "Top", [
      person("p-l1", "Lead 1", [person("p-l1a", "Lead 1A", [person("p-x", "X")])]),
      person("p-l2", "Lead 2", [person("p-l2a", "Lead 2A", [person("p-y", "Y")])]),
    ]);
    const s = resolveScopeRoster(tree, "p-top", { root: null, directOnly: false });
    expect(s.managerNodes.map((m) => m.person_id)).toEqual([
      "p-top",
      "p-l1",
      "p-l1a",
      "p-l2",
      "p-l2a",
    ]);
    expect(s.managerNodes.map((m) => m.teamSize)).toEqual([6, 2, 1, 2, 1]);
  });
});
