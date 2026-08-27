/**
 * One row per event that changed something about the account. What matters: a
 * sync re-observing a binding that never moved is not an event; an operator row
 * always is, even when it moved nobody, because that row is the confirm act;
 * the call is never a row of its own; and its comment is carried across only
 * when the pairing is not a guess, since nothing in the journal links the two
 * records.
 */
import { describe, expect, it } from "vitest";

import type {
  AccountOperation,
  BindingHistoryEntry,
} from "@/api/identity-client";
import { accountTrail } from "./trail";

const ANN = "01900000-0000-7000-8000-0000000000a0";
const BOB = "01900000-0000-7000-8000-0000000000b0";
const OPERATOR = "01900000-0000-7000-8000-0000000000f0";
const AUTOMATION = "00000000-0000-0000-0000-000000000000";

/** A binding row. The wire sends these newest first. */
function seen(
  at: string,
  person: string,
  over: Partial<BindingHistoryEntry> = {},
): BindingHistoryEntry {
  return {
    person_id: person,
    author_person_id: AUTOMATION,
    by_operator: false,
    reason: "",
    recorded_at: at,
    ...over,
  };
}

function decided(
  at: string,
  person: string,
  reason: string,
  over: Partial<BindingHistoryEntry> = {},
): BindingHistoryEntry {
  return seen(at, person, {
    author_person_id: OPERATOR,
    by_operator: true,
    reason,
    ...over,
  });
}

function call(
  at: string,
  verb: string,
  over: Partial<AccountOperation> = {},
): AccountOperation {
  return {
    operation_id: `op-${at}`,
    verb,
    author_person_id: OPERATOR,
    accounts_touched: 1,
    recorded_at: at,
    ...over,
  };
}

/** Newest first, so this reads the trail top-down. */
const persons = (rows: ReturnType<typeof accountTrail>) =>
  rows.map((row) => row.entry.person_id);

describe("accountTrail", () => {
  it("keeps nothing for an account with no journal", () => {
    expect(accountTrail([], [])).toEqual([]);
  });

  // The seed reads the whole connector history every run, so an account that
  // never moved collects one automatic row per sync. Twelve identical rows are
  // one fact.
  it("collapses a binding re-observed by every sync into the one that made it", () => {
    const trail = accountTrail(
      [
        seen("2026-08-03T00:00:00.000000", ANN),
        seen("2026-08-02T00:00:00.000000", ANN),
        seen("2026-08-01T00:00:00.000000", ANN),
      ],
      [],
    );

    expect(trail).toHaveLength(1);
    expect(trail[0].entry.recorded_at).toBe("2026-08-01T00:00:00.000000");
  });

  it("keeps every row where the account changed hands", () => {
    const trail = accountTrail(
      [
        seen("2026-08-04T00:00:00.000000", BOB),
        seen("2026-08-03T00:00:00.000000", BOB),
        seen("2026-08-02T00:00:00.000000", ANN),
        seen("2026-08-01T00:00:00.000000", ANN),
      ],
      [],
    );

    expect(persons(trail)).toEqual([BOB, ANN]);
  });

  // Re-observations of the SEEN person, not of the last kept one: otherwise the
  // sync rows following an operator row come back.
  it("does not resurrect a duplicate after an operator row", () => {
    const trail = accountTrail(
      [
        seen("2026-08-04T00:00:00.000000", ANN),
        decided("2026-08-03T00:00:00.000000", ANN, "operator-bind"),
        seen("2026-08-02T00:00:00.000000", ANN),
      ],
      [],
    );

    expect(trail).toHaveLength(2);
    expect(trail[0].entry.by_operator).toBe(true);
  });

  // Binding an account to the person automation already gave it changes no
  // binding, and IS the decision: it records that a human vouched for the
  // resolver's guess, and the trail is what says who is answerable.
  it("keeps an operator row that moved nobody", () => {
    const trail = accountTrail(
      [
        decided("2026-08-02T00:00:00.000000", ANN, "operator-bind"),
        seen("2026-08-01T00:00:00.000000", ANN),
      ],
      [],
    );

    expect(trail).toHaveLength(2);
    expect(persons(trail)).toEqual([ANN, ANN]);
  });

  it("reads newest first", () => {
    const trail = accountTrail(
      [
        seen("2026-08-02T00:00:00.000000", BOB),
        seen("2026-08-01T00:00:00.000000", ANN),
      ],
      [],
    );

    expect(persons(trail)).toEqual([BOB, ANN]);
  });

  // One call is journalled once and returned on every account it named, so it
  // used to appear beside the row it produced, saying the same thing twice.
  it("never renders a call as a row of its own", () => {
    const trail = accountTrail(
      [decided("2026-08-01T00:00:00.000000", ANN, "operator-bind")],
      [
        call("2026-08-01T00:00:01.000000", "operator-bind", {
          comment: "Same person.",
          accounts_touched: 110,
        }),
      ],
    );

    expect(trail).toHaveLength(1);
    expect(trail[0].comment).toBe("Same person.");
  });

  // A call that changed nothing here — already decided, refused, or an account
  // a merge merely swept along — has no row to carry it.
  it("drops a call that produced no movement here", () => {
    const trail = accountTrail(
      [],
      [
        call("2026-08-01T00:00:00.000000", "operator-merge", {
          comment: "Merging the other one.",
          accounts_touched: 30,
          outcome: "already_decided",
        }),
      ],
    );

    expect(trail).toEqual([]);
  });

  it("carries each comment to its own decision when the calls line up", () => {
    const trail = accountTrail(
      [
        decided("2026-08-02T00:00:00.000000", BOB, "operator-bind"),
        decided("2026-08-01T00:00:00.000000", ANN, "operator-bind"),
      ],
      [
        call("2026-08-02T00:00:01.000000", "operator-bind", { comment: "second" }),
        call("2026-08-01T00:00:01.000000", "operator-bind", { comment: "first" }),
      ],
    );

    expect(trail.map((row) => row.comment)).toEqual(["second", "first"]);
  });

  // Nothing links a binding row to the call that wrote it, so an unequal count
  // means at least one call wrote nothing here — and any pairing would be a
  // guess. A missing comment is recoverable; a comment on the wrong decision is
  // a false audit trail.
  it("attributes nothing when the counts disagree", () => {
    const trail = accountTrail(
      [
        decided("2026-08-02T00:00:00.000000", BOB, "operator-bind"),
        decided("2026-08-01T00:00:00.000000", ANN, "operator-bind"),
      ],
      [call("2026-08-02T00:00:01.000000", "operator-bind", { comment: "which?" })],
    );

    expect(trail.map((row) => row.comment)).toEqual([undefined, undefined]);
  });

  // The verb and the author are all the two records share, so they are what the
  // groups are cut on: a detach's comment must not caption a bind.
  it("keeps the verbs and the authors apart", () => {
    const trail = accountTrail(
      [
        decided("2026-08-02T00:00:00.000000", BOB, "operator-detach"),
        decided("2026-08-01T00:00:00.000000", ANN, "operator-bind", {
          author_person_id: "01900000-0000-7000-8000-0000000000f1",
        }),
      ],
      [
        call("2026-08-02T00:00:01.000000", "operator-detach", { comment: "not theirs" }),
        call("2026-08-01T00:00:01.000000", "operator-bind", { comment: "somebody else's call" }),
      ],
    );

    expect(trail[0].comment).toBe("not theirs");
    // The bind was authored by a different operator, so its group has one
    // decision and no call.
    expect(trail[1].comment).toBeUndefined();
  });

  it("treats a blank comment as no comment", () => {
    const trail = accountTrail(
      [decided("2026-08-01T00:00:00.000000", ANN, "operator-bind")],
      [call("2026-08-01T00:00:01.000000", "operator-bind", { comment: "   " })],
    );

    expect(trail[0].comment).toBeUndefined();
  });

  it("carries no comment to an automatic row", () => {
    const trail = accountTrail(
      [seen("2026-08-01T00:00:00.000000", ANN, { reason: "auto-seed-link" })],
      [
        {
          operation_id: "op-1",
          verb: "auto-seed-link",
          author_person_id: AUTOMATION,
          accounts_touched: 1,
          comment: "should not appear",
          recorded_at: "2026-08-01T00:00:01.000000",
        },
      ],
    );

    expect(trail[0].comment).toBeUndefined();
  });

  it("survives a read that carries no operations array at all", () => {
    const trail = accountTrail(
      [decided("2026-08-01T00:00:00.000000", ANN, "operator-bind")],
      undefined,
    );

    expect(trail).toHaveLength(1);
    expect(trail[0].comment).toBeUndefined();
  });
});
