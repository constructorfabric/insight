import { describe, expect, it } from "vitest";

import { seriesColors } from "@/lib/series-colors";

describe("seriesColors", () => {
  it("is deterministic regardless of duplicate seed inputs", () => {
    expect(seriesColors(["cursor", "claude_code", "cursor"])).toEqual(
      seriesColors(["claude_code", "cursor"])
    );
  });

  it("returns one color per unique seed", () => {
    expect(
      Object.keys(seriesColors(["cursor", "cursor", "codex"])).sort()
    ).toEqual(["codex", "cursor"]);
  });

  it("gives known tools their brand token", () => {
    const palette = seriesColors(["slack", "m365", "zoom", "zulip_proxy"]);
    expect(palette.slack).toBe("var(--brand-slack)");
    expect(palette.m365).toBe("var(--brand-m365)");
    expect(palette.zoom).toBe("var(--brand-zoom)");
    expect(palette.zulip_proxy).toBe("var(--brand-zulip)");
  });

  it("assigns chart tokens to unknown seeds in sorted order", () => {
    const palette = seriesColors(["beta", "alpha"]);
    expect(palette.alpha).toBe("var(--chart-1)");
    expect(palette.beta).toBe("var(--chart-2)");
  });

  it("does not skip chart slots for brand-mapped seeds", () => {
    const palette = seriesColors(["slack", "unknown_tool"]);
    expect(palette.slack).toBe("var(--brand-slack)");
    expect(palette.unknown_tool).toBe("var(--chart-1)");
  });

  it("cycles chart tokens past twelve unknown seeds", () => {
    const seeds = [
      "a",
      "b",
      "c",
      "d",
      "e",
      "f",
      "g",
      "h",
      "i",
      "j",
      "k",
      "l",
      "m",
    ];
    const palette = seriesColors(seeds);
    expect(palette.a).toBe("var(--chart-1)");
    expect(palette.l).toBe("var(--chart-12)");
    expect(palette.m).toBe("var(--chart-1)");
  });

  it("treats prototype-member seeds as unknown, never as brand colors", () => {
    const palette = seriesColors(["__proto__", "constructor"]);
    expect(palette["__proto__"]).toBe("var(--chart-1)");
    expect(palette["constructor"]).toBe("var(--chart-2)");
  });
});
