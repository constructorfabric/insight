import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn } from "storybook/test";

import { RangePicker } from "./range-picker";

const meta: Meta<typeof RangePicker> = {
  title: "Custom/RangePicker",
  component: RangePicker,
  args: {
    offered: ["PDC", "P7D", "P30D"],
    selected: "P30D",
    onSelect: fn(),
  },
  decorators: [
    (Story) => (
      <div className="bg-background p-2">
        <Story />
      </div>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof RangePicker>;

function paintedBackground(element: Element | null): string {
  for (let node = element; node; node = node.parentElement) {
    const color = getComputedStyle(node).backgroundColor;
    if (color !== "rgba(0, 0, 0, 0)") return color;
  }
  return "rgba(0, 0, 0, 0)";
}

export const Default: Story = {};

export const TestSelectedRangeStandsOut: Story = {
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const items = [
      ...canvasElement.querySelectorAll<HTMLElement>("[aria-pressed]"),
    ];
    const selected = items.filter(
      (item) => item.getAttribute("aria-pressed") === "true"
    );
    const surface = paintedBackground(
      canvasElement.querySelector("[data-slot=toggle-group]")?.parentElement ??
        null
    );

    await expect(selected).toHaveLength(1);
    await expect(paintedBackground(selected[0]!)).not.toBe(surface);
    for (const item of items.filter((i) => i !== selected[0])) {
      await expect(paintedBackground(item)).not.toBe(
        paintedBackground(selected[0]!)
      );
    }
  },
};
