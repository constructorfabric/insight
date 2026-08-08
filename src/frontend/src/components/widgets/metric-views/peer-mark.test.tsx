// @vitest-environment jsdom
/**
 * The mark that says how unlike the group one reading is.
 *
 * Its whole job is to be ignorable when the answer is "like everyone else"
 * and unmissable when it is not, so these tests are about the three ways that
 * fails: a mark where nothing is comparable, colour where no verdict was
 * earned, and one extreme reading squashing the axis for every other row.
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PeerMark } from "@/components/widgets/metric-views/peer-mark";
import type { PeerStanding } from "@/lib/metrics/peer-standing";
import type { PeerStatusWithNeutral } from "@/lib/peers";

function standing(
  overrides: Partial<PeerStanding> & { gapDelta?: number } = {}
): PeerStanding {
  return {
    observed: true,
    stats: { p25: 8, p50: 10, p75: 12, min: 0, max: 30, n: 20 },
    eligible: true,
    reason: "ok",
    rank: "in_pack" as PeerStatusWithNeutral,
    gapDelta: 0,
    gapPct: 0,
    severity: 0,
    spreadGap: 0,
    ...overrides,
  } as PeerStanding;
}

function draw(s: PeerStanding) {
  const { container } = render(
    <PeerMark standing={s} metricLabel="Commits" format="integer" />
  );
  return {
    dot: container.querySelector("circle"),
    pin: container.querySelector("path"),
    svg: container.querySelector('[data-slot="peer-mark"]'),
    label: container
      .querySelector('[data-slot="peer-mark"]')
      ?.getAttribute("aria-label"),
  };
}

describe("PeerMark", () => {
  it("puts a reading at the group's middle on the shared line", () => {
    // The axis is 112 wide, so the middle is 56 — the same x every row draws
    // its line at. Ordinary readings are meant to disappear into it.
    const { dot } = draw(standing({ gapDelta: 0 }));
    expect(Number(dot?.getAttribute("cx"))).toBe(56);
  });

  it("measures distance in spreads, not in the metric's own units", () => {
    // IQR here is 4, so a gap of 4 is one spread — and one spread means the
    // same thing on a row counting hours and a row counting messages.
    const { dot, label } = draw(standing({ gapDelta: 4 }));
    expect(label).toMatch(/1\.0 spreads above/);
    // One spread of 2.5 on the right half: 56 + (1/2.5) * 51.
    expect(Number(dot?.getAttribute("cx"))).toBeCloseTo(76.4, 1);
  });

  it("draws no mark where there is nothing to compare against", () => {
    // But keeps the element, so the row holds its height and the shared line
    // behind it stays unbroken. An absent dot means "not comparable" — it
    // must not be confused with a dot sitting at the middle.
    const { dot, pin, svg } = draw(
      standing({ eligible: false, rank: "neutral", gapDelta: 3 })
    );
    expect(svg).not.toBeNull();
    expect(dot).toBeNull();
    expect(pin).toBeNull();
  });

  it("pins an extreme reading instead of letting it set the scale", () => {
    // Eleven spreads out would otherwise compress every other row into the
    // middle. The shape changes so the edge is not read as a measurement.
    const { dot, pin, label } = draw(standing({ gapDelta: 44 }));
    expect(dot).toBeNull();
    expect(pin).not.toBeNull();
    expect(label).toMatch(/11\.0 spreads above/);
  });

  it("colours only what the shared standing calls bottom of the pack", () => {
    // Standing out is not the same as being wrong: a reading far above the
    // middle is remarkable on any metric and a problem on almost none.
    const far = draw(standing({ gapDelta: 8, rank: "top" }));
    expect(far.dot?.getAttribute("class")).not.toMatch(/destructive/);

    const adverse = draw(standing({ gapDelta: -8, rank: "bottom" }));
    expect(adverse.dot?.getAttribute("class")).toMatch(/destructive/);
  });
});
