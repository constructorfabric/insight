/**
 * PoC story + component test for <KpiTile>.
 *
 * Demonstrates the Storybook component-test setup end-to-end. Note the
 * @storybook/addon-vitest model as configured here (`tags.include: ["test"]`
 * in vitest.config.ts): a story runs as a browser test only when it carries
 * the `test` tag. A tagged story with no `play` is a smoke test (renders
 * without throwing); a tagged story with `play` adds interaction assertions.
 *
 *   - `Default`      — demo story for the Storybook UI (untagged; not a test).
 *   - `TestOkTile`   — play-driven test asserting the tile renders the
 *                      display-ready selector output (value, delta, median).
 *   - `TestNoPeers`  — a tile whose selector produced no median (server-side
 *                      suppression or no peer data) falls back in the footer.
 *
 * KpiTile is presentational: selectors in `lib/insight/kpi-row.ts` own all
 * formatting and scoring, so these stories feed the tile intermediate
 * directly — no wire mocking involved.
 *
 * See docs/testing/storybook-component-tests.md.
 */

import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";

import type { KpiTileData } from "@/lib/insight/kpi-row";

import { KpiTile } from "./kpi-tile";

function makeTile(overrides: Partial<KpiTileData> = {}): KpiTileData {
  return {
    key: "tasks.closed",
    label: "Bugs Fixed",
    value: "12",
    valueStatus: "good",
    delta: { text: "+9%", status: "good", down: false },
    medianLabel: "median 6",
    gapText: null,
    gapStatus: "neutral",
    context: "Jira",
    groupId: "task_delivery",
    ...overrides,
  };
}

const meta: Meta<typeof KpiTile> = {
  title: "Widgets/Dashboard/KpiTile",
  component: KpiTile,
  // In the app KpiTile is a grid cell sized by its parent; the card itself has
  // no intrinsic width (its labels use `truncate`, i.e. `white-space: nowrap`),
  // so in an isolated `layout: "centered"` story it would collapse to width 0
  // and render as a blank canvas. Give it a representative tile width (above
  // the `@[250px]/card` container-query breakpoint) so it renders as it does
  // in a real dashboard.
  decorators: [
    (Story) => (
      <div className="w-72">
        <Story />
      </div>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof KpiTile>;

/** Demo story for the Storybook UI (not a test — no `test` tag). */
export const Default: Story = {
  args: { tile: makeTile() },
};

/** Component test: the display-ready tile input drives every element. */
export const TestOkTile: Story = {
  tags: ["test"],
  args: { tile: makeTile() },
  play: async ({ canvas }) => {
    // Singular `getByText` (throws on >1 match) doubles as a guard that the
    // preview decorators wrap the story exactly once — a double-applied
    // decorator would render two <KpiTile>s and fail here.
    await expect(canvas.getByText("Bugs Fixed")).toBeInTheDocument();
    await expect(canvas.getByText("12")).toBeInTheDocument();
    await expect(canvas.getByText("+9%")).toBeInTheDocument();
    await expect(canvas.getByText(/median 6/i)).toBeInTheDocument();
  },
};

/**
 * Component test: a tile without a median (selector suppressed it — thin
 * cohort server-side, schema error, or no peer data) falls back in the
 * footer and renders the value without peer coloring.
 */
export const TestNoPeers: Story = {
  tags: ["test"],
  args: {
    tile: makeTile({ valueStatus: "neutral", medianLabel: null, delta: null }),
  },
  play: async ({ canvas }) => {
    await expect(canvas.getByText("Bugs Fixed")).toBeInTheDocument();
    await expect(canvas.getByText(/no peer data/i)).toBeInTheDocument();
    await expect(canvas.queryByText(/median/i)).not.toBeInTheDocument();
  },
};
