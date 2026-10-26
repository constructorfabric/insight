import { describe, expect, it } from "vitest";

import { CustomApiError } from "@/api/custom-client";

import { refusal } from "./refusal";

const FALLBACK = "Couldn't do it.";
const GENERIC = "You do not have permission to perform this operation";

const denied = (reason: string) =>
  new CustomApiError(403, { detail: GENERIC, context: { reason } });

describe("refusal", () => {
  it.each([
    [
      "only the owner or an editor can change this dashboard",
      "Only the owner or an editor can change this dashboard.",
    ],
    [
      "only the owner can delete this dashboard",
      "Only the owner can delete this dashboard.",
    ],
    [
      "only the owner or an editor granted this dashboard directly or through everyone can share it",
      "Editors through a folder can't share this dashboard.",
    ],
    [
      "only the owner or an editor can change this folder",
      "Only the owner or an editor can change this folder.",
    ],
    [
      "only the owner can delete this folder",
      "Only the owner can delete this folder.",
    ],
    [
      "admin role required for this operation",
      "Only an administrator can do this.",
    ],
  ])("words a known denial %j as %j", (reason, sentence) => {
    expect(refusal(denied(reason), FALLBACK), `reason: ${reason}`).toBe(
      sentence
    );
  });

  it("falls back to the detail for a denial it does not know", () => {
    expect(refusal(denied("a reason added later"), FALLBACK)).toBe(GENERIC);
  });

  it("does not read a reason the prototype carries as a known one", () => {
    expect(refusal(denied("constructor"), FALLBACK)).toBe(GENERIC);
  });

  it.each([
    [
      "a field violation",
      {
        detail: "Invalid",
        context: { field_violations: [{ description: "name is empty" }] },
      },
      "name is empty",
    ],
    [
      "a precondition violation",
      {
        detail: "Conflict",
        context: { violations: [{ description: "it is in use" }] },
      },
      "it is in use",
    ],
    ["a bare detail", { detail: "Not found" }, "Not found"],
    ["no body", null, FALLBACK],
  ])("reads %s", (_case, body, shown) => {
    expect(
      refusal(new CustomApiError(400, body), FALLBACK),
      `case: ${_case}`
    ).toBe(shown);
  });

  it("uses the fallback for an error that is not the service's", () => {
    expect(refusal(new Error("network"), FALLBACK)).toBe(FALLBACK);
  });
});
