import { describe, expect, it } from "vitest";

import { hiddenHolders } from "./hidden-holders";

describe("hiddenHolders", () => {
  it.each([
    [0, 2, null],
    [-1, 0, null],
    [1, 1, "and 1 dashboard you can't see"],
    [3, 2, "and 3 dashboards you can't see"],
    [1, 0, "1 dashboard you can't see"],
    [2, 0, "2 dashboards you can't see"],
  ])("says %s unseen beside %s seen as %s", (hidden, visible, said) => {
    expect(
      hiddenHolders(hidden, visible),
      `hidden: ${hidden}, visible: ${visible}`
    ).toBe(said);
  });
});
