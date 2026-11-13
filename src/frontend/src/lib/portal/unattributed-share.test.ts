import { describe, expect, it } from "vitest";

import {
  showsUnattributedShare,
  type UnattributedScope,
} from "@/lib/portal/unattributed-share";

const wholeOrganisation: UnattributedScope = {
  personal: false,
  scoped: false,
  mayReadOrganisation: true,
  scopeMemberCount: 12,
  visibleCount: 12,
};

describe("showsUnattributedShare", () => {
  it("shows the share when the lens sums everyone the viewer may see", () => {
    expect(showsUnattributedShare(wholeOrganisation)).toBe(true);
  });

  it.each<[string, Partial<UnattributedScope>]>([
    ["a single person", { personal: true }],
    ["a lens narrowed to one dimension value", { scoped: true }],
    ["a viewer the backend refuses", { mayReadOrganisation: false }],
    ["a subtree smaller than the organisation", { scopeMemberCount: 5 }],
    ["a roster still loading", { scopeMemberCount: 0, visibleCount: 0 }],
  ])("hides the share for %s", (_name, override) => {
    expect(
      showsUnattributedShare({ ...wholeOrganisation, ...override }),
      `should hide for ${_name}`
    ).toBe(false);
  });
});
