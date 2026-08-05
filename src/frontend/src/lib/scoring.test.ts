import { describe, expect, it } from "vitest";

import type { PeerStatusWithNeutral } from "@/lib/peers";
import {
  gradeSectionStanding,
  rankCounts,
  rankableCount,
  sectionStandingPhrase,
  type RankCounts,
  type RankedMetric,
} from "@/lib/scoring";

const ranked = (rank: PeerStatusWithNeutral, id: string = rank): RankedMetric<string> => ({
  row: id,
  rank,
});

const counts = (over: Partial<RankCounts>): RankCounts => ({
  top: 0,
  inPack: 0,
  bottom: 0,
  unranked: 0,
  ...over,
});

describe("rankCounts / rankableCount", () => {
  it("tallies each rank, sending neutral to unranked", () => {
    const c = rankCounts([
      ranked("top"),
      ranked("top"),
      ranked("in_pack"),
      ranked("bottom"),
      ranked("neutral"),
    ]);
    expect(c).toEqual({ top: 2, inPack: 1, bottom: 1, unranked: 1 });
    expect(rankableCount(c)).toBe(4);
  });
});

describe("gradeSectionStanding", () => {
  it("stays neutral with nothing rankable", () => {
    expect(gradeSectionStanding(counts({ unranked: 3 }))).toBe("neutral");
  });

  it("goes bad only on a pattern of weakness (≥2 bottoms above the bar)", () => {
    expect(gradeSectionStanding(counts({ bottom: 2, inPack: 2 }))).toBe("bad");
  });

  it("keeps a single bottom at warn", () => {
    expect(gradeSectionStanding(counts({ bottom: 1, inPack: 5 }))).toBe("warn");
  });

  it("keeps two bottoms in a large section at warn (below the pattern bar)", () => {
    expect(gradeSectionStanding(counts({ bottom: 2, inPack: 8 }))).toBe("warn");
  });

  it("goes good on a mirrored pattern of strength", () => {
    expect(gradeSectionStanding(counts({ top: 2, inPack: 2 }))).toBe("good");
  });

  it("stays neutral when tops are too diluted or the pack rules", () => {
    expect(gradeSectionStanding(counts({ top: 2, inPack: 8 }))).toBe("neutral");
    expect(gradeSectionStanding(counts({ inPack: 4 }))).toBe("neutral");
    expect(gradeSectionStanding(counts({ top: 1, inPack: 1 }))).toBe("neutral");
  });
});

describe("sectionStandingPhrase", () => {
  it("says no peer data with nothing rankable", () => {
    expect(sectionStandingPhrase(counts({ unranked: 2 }))).toBe("no peer data");
  });

  it("prefers behind over ahead on mixed profiles", () => {
    expect(sectionStandingPhrase(counts({ bottom: 2, top: 3 }))).toBe(
      "2 behind peers",
    );
  });

  it("reports ahead when nothing is behind", () => {
    expect(sectionStandingPhrase(counts({ top: 3, inPack: 1 }))).toBe(
      "3 ahead of peers",
    );
  });

  it("reports on par for an all-in-pack section", () => {
    expect(sectionStandingPhrase(counts({ inPack: 4 }))).toBe(
      "on par with peers",
    );
  });
});

