import { describe, expect, it } from "vitest";

import { hiddenHolders } from "./hidden-holders";

describe("hiddenHolders", () => {
  it.each([
    [0, null],
    [-1, null],
    [1, "and 1 dashboard you can't see"],
    [3, "and 3 dashboards you can't see"],
  ])("says %s unseen dashboards as %s", (hidden, said) => {
    expect(hiddenHolders(hidden), `hidden: ${hidden}`).toBe(said);
  });
});
