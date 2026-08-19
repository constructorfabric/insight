import { describe, expect, it } from "vitest";

import { filterOrgTree } from "@/lib/portal/org-tree-filter";
import type { IdentityPerson } from "@/types/insight";

function person(
  id: string,
  fields: Partial<IdentityPerson> = {},
  subordinates: IdentityPerson[] = []
): IdentityPerson {
  return {
    person_id: id,
    email: `${id}@example.com`,
    display_name: id,
    subordinates,
    ...fields,
  };
}

//  root
//  ├── lead        (Engineering)
//  │   ├── deep    (job title: Parser wrangler)
//  │   └── other
//  └── peer
const tree = person("root", { display_name: "Root Person" }, [
  person("lead", { display_name: "Lead Person", department: "Engineering" }, [
    person("deep", { display_name: "Deep Person", job_title: "Parser wrangler" }),
    person("other", { display_name: "Other Person" }),
  ]),
  person("peer", { display_name: "Peer Person" }),
]);

describe("filterOrgTree", () => {
  it("does not filter a blank query", () => {
    expect(filterOrgTree(tree, "")).toBeNull();
    expect(filterOrgTree(tree, "   ")).toBeNull();
    expect(filterOrgTree(null, "deep")).toBeNull();
  });

  it("keeps the managers that reach a match", () => {
    const result = filterOrgTree(tree, "Deep");
    expect([...result!.visible].sort()).toEqual(["deep", "lead", "root"]);
    expect([...result!.matched]).toEqual(["deep"]);
  });

  it("leaves a matched manager's reports out unless they match too", () => {
    const result = filterOrgTree(tree, "Lead");
    expect(result!.visible.has("lead")).toBe(true);
    expect(result!.visible.has("deep")).toBe(false);
    expect(result!.visible.has("other")).toBe(false);
  });

  it("matches the fields the roster searches, not just the name", () => {
    expect(filterOrgTree(tree, "engineering")!.matched.has("lead")).toBe(true);
    expect(filterOrgTree(tree, "parser wrangler")!.matched.has("deep")).toBe(
      true
    );
  });

  it("ignores case and surrounding space", () => {
    expect(filterOrgTree(tree, "  dEeP  ")!.matched.has("deep")).toBe(true);
  });

  it("reports nothing visible when no one matches", () => {
    const result = filterOrgTree(tree, "nobody here");
    expect(result!.visible.size).toBe(0);
    expect(result!.matched.size).toBe(0);
  });

  it("keeps two separate branches when both hold a match", () => {
    const result = filterOrgTree(tree, "Person");
    expect([...result!.visible].sort()).toEqual([
      "deep",
      "lead",
      "other",
      "peer",
      "root",
    ]);
  });
});
