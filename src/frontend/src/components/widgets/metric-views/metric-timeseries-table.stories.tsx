/**
 * Browser component tests for the geometry of `<MetricTimeseriesTable>` — the
 * scroll controls and the room they leave the data.
 *
 * What the table renders, and where a page scrolls to, is covered by the jsdom
 * tests in `metric-timeseries-presentations.test.tsx`, which hand it fake
 * geometry. What needs a real browser is the geometry itself: the controls sit
 * in gutters taken out of the scrollport, and both the sticky bucket column and
 * a whole value column have to survive that on a phone.
 *
 * `metric-timeseries-view.stories.tsx` covers the same table inside its card;
 * this file renders it in a box of a chosen size, which is the only way to
 * reach the short-table case.
 *
 * See docs/testing/storybook-component-tests.md.
 */

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, waitFor } from "storybook/test";

import { MetricTimeseriesTable } from "@/components/widgets/metric-views/metric-timeseries-table";
import { groupedTimeseriesModel } from "@/components/widgets/metric-views/metric-timeseries.test-fixtures";
import {
  CARD_PX_AT_390,
  expectNothingOverlaps,
} from "@/test/storybook/layout";

/** Short enough that the three rows overrun it, so the row controls render. */
const SHORT_PX = 180;

/** Mirrors the gutter the component reserves — see the widening story. */
const GUTTER_PX = 48;

const meta: Meta<typeof MetricTimeseriesTable> = {
  title: "Widgets/MetricViews/MetricTimeseriesTable",
  component: MetricTimeseriesTable,
  args: { model: groupedTimeseriesModel() },
};
export default meta;

type Story = StoryObj<typeof MetricTimeseriesTable>;

/** Demo story for the Storybook UI (untagged — not a test). */
export const Default: Story = {
  decorators: [
    (Story) => (
      <div style={{ width: 900, height: 300 }}>
        <Story />
      </div>
    ),
  ],
};

function scrollControls(root: HTMLElement): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>('button[aria-label^="Show "]')
  );
}

function nameOf(element: Element): string {
  return element.getAttribute("aria-label") ?? "a scroll control";
}

/**
 * Scrolling both ways, the four controls clear the cells and each other.
 *
 * The row pair shares the end gutter with the column control, so this is the
 * arrangement that stops being obvious: three chevrons in one lane, held apart
 * only by the height the caller gives the table.
 */
export const TestShortTableKeepsItsControlsApart: Story = {
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_AT_390, height: SHORT_PX }}>
        <Story />
      </div>
    ),
  ],
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const table = canvasElement.querySelector("table")!;
    const scrollport = table.parentElement!;
    const controls = scrollControls(canvasElement);

    const labels = controls.map(nameOf);
    await expect(
      labels,
      `the table scrolls both ways, so it renders a control for each (${labels.join(", ")})`
    ).toEqual(
      expect.arrayContaining(["Show later columns", "Show later rows"])
    );

    expectNothingOverlaps(scrollport, controls, nameOf);
    for (const control of controls) {
      expectNothingOverlaps(
        control,
        controls.filter((other) => other !== control),
        nameOf
      );
    }
  },
};

/**
 * On a phone the bucket column and a whole value column both still read.
 *
 * Moving the controls off the cells costs the scrollport two gutters, and the
 * sticky bucket column is taken out of what is left. Clearing the cells by
 * taking the room they needed would leave the table exactly as unreadable as
 * the controls painted over it did.
 */
export const TestNarrowTableLeavesRoomForAValueColumn: Story = {
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_AT_390, height: 400 }}>
        <Story />
      </div>
    ),
  ],
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const table = canvasElement.querySelector("table")!;
    const scrollport = table.parentElement!;
    const cells = Array.from(
      table.querySelectorAll<HTMLElement>("tbody tr:first-child > *")
    );

    await expect(
      cells.length,
      "the first row has a bucket cell and at least one value"
    ).toBeGreaterThan(1);

    const room = scrollport.clientWidth;
    const bucket = cells[0].getBoundingClientRect().width;
    const value = cells[1].getBoundingClientRect().width;
    await expect(
      Math.round(bucket + value),
      `the bucket column (${Math.round(bucket)}px) and the first value ` +
        `(${Math.round(value)}px) do not fit the ${room}px scrollport`
    ).toBeLessThanOrEqual(room);

    // The date is what the bucket column is narrow for; a column that clips it
    // would satisfy the width check and tell the reader nothing.
    const date = cells[0];
    await expect(
      date.scrollWidth,
      `the bucket cell clips its date (${date.scrollWidth}px of text in ${date.clientWidth}px)`
    ).toBeLessThanOrEqual(date.clientWidth);
  },
};

/**
 * Given the room back, the table gives the gutters back.
 *
 * The gutters are the wrapper's padding, so measuring the overflow on the
 * scrollport feeds them their own effect: the width they took makes the
 * overflow that keeps them. That latches — a card that was ever narrow keeps a
 * sideways scroll and two empty gutters at every width afterwards — and only a
 * resize in both directions catches it.
 */
export const TestWidenedTableReleasesItsGutters: Story = {
  decorators: [
    (Story) => (
      <div data-testid="box" style={{ width: CARD_PX_AT_390, height: 400 }}>
        <Story />
      </div>
    ),
  ],
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const box = canvasElement.querySelector<HTMLElement>('[data-testid="box"]')!;
    const table = canvasElement.querySelector("table")!;
    const wrapper = table.parentElement!.parentElement!;

    await waitFor(() =>
      expect(
        scrollControls(canvasElement).length,
        "the narrow table scrolls sideways to begin with"
      ).toBeGreaterThan(0)
    );

    // One gutter's worth of room past the table is the width that catches a
    // latch and nothing else does: wide enough that the table fits with no
    // gutters, narrow enough that it does not once a stale pair is subtracted.
    box.style.width = `${table.offsetWidth + GUTTER_PX}px`;

    await waitFor(() =>
      expect(
        scrollControls(canvasElement).map(nameOf),
        "the widened table still offers to scroll"
      ).toEqual([])
    );

    const scrollport = table.parentElement!;
    await expect(
      scrollport.scrollWidth,
      `the widened table still scrolls sideways (${scrollport.scrollWidth}px of table in ${scrollport.clientWidth}px)`
    ).toBeLessThanOrEqual(scrollport.clientWidth);
    await expect(
      wrapper.className,
      "the wrapper kept a gutter it no longer has a control for"
    ).not.toContain("ps-12");
  },
};
