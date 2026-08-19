import { describe, expect, it } from "vitest";

import en from "@/locales/en/translation.json";

import { MODE_LABELS, MODES } from "./identity-modes";

describe("MODE_LABELS", () => {
  it("says what the console's own tabs say", () => {
    for (const mode of MODES) {
      expect(MODE_LABELS[mode]).toBe(
        (en as { identities: { modes: Record<string, string> } }).identities.modes[mode],
      );
    }
  });
});
