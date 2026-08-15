/**
 * A conflict flags every account sharing the disputed value, each row
 * carrying the same candidates — so the queue showed one split person as five
 * problems, and ordering by source scattered them among the others. Grouping
 * by "who is being argued over" is what puts one case back together.
 */
import { describe, expect, it } from "vitest";

import type { AttentionItem } from "@/api/identity-client";

import { dropDecided, filterQueue, groupIntoCases } from "./cases";

const ANN = { person_id: "01900000-0000-7000-8000-0000000000a0", display_name: "Ann Lee" };
const BOB = { person_id: "01900000-0000-7000-8000-0000000000b0", display_name: "Bob Park" };

function item(over: Partial<AttentionItem>): AttentionItem {
  return {
    kind: "binding_conflict",
    source: "github",
    source_id: "01900000-0000-7000-8000-00000000aa01",
    account_id: "a1",
    email: "dev@example.com",
    username: null,
    candidates: [ANN, BOB],
    ...over,
  };
}

describe("groupIntoCases", () => {
  it("collects every account arguing over the same people into one case", () => {
    const cases = groupIntoCases([
      item({ account_id: "a1", source: "github" }),
      item({ account_id: "a2", source: "gitlab" }),
      item({ account_id: "a3", source: "hr" }),
    ]);

    expect(cases).toHaveLength(1);
    expect(cases[0]?.items).toHaveLength(3);
    expect(cases[0]?.candidates).toEqual([ANN, BOB]);
  });

  // The server sorts candidates, but a case must not split because a future
  // read hands the same two people back in the other order.
  it("is blind to the order the candidates arrive in", () => {
    const cases = groupIntoCases([
      item({ account_id: "a1", candidates: [ANN, BOB] }),
      item({ account_id: "a2", candidates: [BOB, ANN] }),
    ]);

    expect(cases).toHaveLength(1);
  });

  it("keeps separate arguments apart", () => {
    const cases = groupIntoCases([
      item({ account_id: "a1", candidates: [ANN, BOB] }),
      item({ account_id: "a2", candidates: [ANN] }),
    ]);

    expect(cases).toHaveLength(2);
  });

  // Nothing to match on means nothing to join them by: collapsing these would
  // claim an argument that does not exist.
  it("leaves candidate-less accounts as cases of their own", () => {
    const cases = groupIntoCases([
      item({ account_id: "a1", kind: "no_evidence", candidates: [] }),
      item({ account_id: "a2", kind: "no_evidence", candidates: [] }),
    ]);

    expect(cases).toHaveLength(2);
  });
});

describe("filterQueue", () => {
  it("matches the account's own values and its source", () => {
    const items = [
      item({ account_id: "a1", email: "ann@example.com", source: "hr" }),
      item({ account_id: "a2", email: "bob@example.com", source: "wiki" }),
    ];

    expect(filterQueue(items, "wiki").map((i) => i.account_id)).toEqual(["a2"]);
    expect(filterQueue(items, "ann@").map((i) => i.account_id)).toEqual(["a1"]);
  });

  // An operator hunting the accounts of one split person searches by the
  // person — and by the id they copied off the card, which is the one term
  // that cannot land on a namesake.
  it("matches a candidate by name and by person id", () => {
    const items = [
      item({ account_id: "a1", candidates: [ANN] }),
      item({ account_id: "a2", candidates: [BOB] }),
    ];

    expect(filterQueue(items, "ann lee").map((i) => i.account_id)).toEqual(["a1"]);
    expect(filterQueue(items, BOB.person_id).map((i) => i.account_id)).toEqual(["a2"]);
  });

  it("requires every term, so a second word narrows rather than widens", () => {
    const items = [
      item({ account_id: "a1", source: "hr", candidates: [ANN] }),
      item({ account_id: "a2", source: "wiki", candidates: [ANN] }),
    ];

    expect(filterQueue(items, "ann wiki").map((i) => i.account_id)).toEqual(["a2"]);
  });

  it("returns everything for a blank query", () => {
    const items = [item({ account_id: "a1" })];

    expect(filterQueue(items, "   ")).toHaveLength(1);
  });
});

describe("dropDecided", () => {
  const ref = { source: "github", source_id: "01900000-0000-7000-8000-00000000aa01" };

  it("removes the accounts the server reported as decided", () => {
    const items = [item({ account_id: "a1" }), item({ account_id: "a2" })];

    const left = dropDecided(items, [
      { ...ref, account_id: "a1", outcome: "applied" },
    ]);

    expect(left.map((i) => i.account_id)).toEqual(["a2"]);
  });

  it("treats an already-decided account as decided too", () => {
    const items = [item({ account_id: "a1" })];

    expect(
      dropDecided(items, [{ ...ref, account_id: "a1", outcome: "already_decided" }]),
    ).toHaveLength(0);
  });

  // A refusal changed nothing, so the row must stay: removing it would show a
  // queue that dealt with something the server declined.
  it("keeps a refused account in the queue", () => {
    const items = [item({ account_id: "a1" })];

    expect(
      dropDecided(items, [{ ...ref, account_id: "a1", outcome: "refused" }]),
    ).toHaveLength(1);
  });

  it("matches on the whole account triple, not the id alone", () => {
    const items = [item({ account_id: "a1", source: "gitlab" })];

    expect(
      dropDecided(items, [{ ...ref, account_id: "a1", outcome: "applied" }]),
    ).toHaveLength(1);
  });
});
