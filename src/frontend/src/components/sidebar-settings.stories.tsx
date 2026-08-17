import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";

import { SidebarProvider } from "@/components/ui/sidebar";

import { SidebarSettings } from "./sidebar-settings";

// INVARIANT: `w-64 p-1` mirrors the rail's settings popover (lens-rail.tsx),
// the narrowest surface these controls render in.
const meta: Meta<typeof SidebarSettings> = {
  title: "Components/SidebarSettings",
  component: SidebarSettings,
  decorators: [
    (Story) => (
      <SidebarProvider>
        <div className="w-64 p-1">
          <Story />
        </div>
      </SidebarProvider>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof SidebarSettings>;

function contentWidth(element: HTMLElement): number {
  const style = getComputedStyle(element);
  return (
    element.clientWidth -
    parseFloat(style.paddingInlineStart) -
    parseFloat(style.paddingInlineEnd)
  );
}

export const Default: Story = {};

export const TestToggleRowsShowTheirWholeLabel: Story = {
  tags: ["test"],
  play: async ({ canvas }) => {
    for (const name of [/portal/i, /planned sections/i, /explanations/i]) {
      const row = canvas.getByRole("button", { name });
      const children = [...row.children];
      const gap = parseFloat(getComputedStyle(row).columnGap);
      const used = children.reduce(
        (total, child) => total + child.getBoundingClientRect().width,
        gap * (children.length - 1),
      );
      const label = row.querySelector<HTMLElement>("span > span")!;

      await expect(used).toBeLessThanOrEqual(contentWidth(row));
      await expect(label.scrollWidth).toBeLessThanOrEqual(label.clientWidth);
    }
  },
};

export const TestFocusLabelsFitTheirSegments: Story = {
  tags: ["test"],
  play: async ({ canvas }) => {
    for (const name of ["Critical", "Rewards", "Calm", "All"]) {
      const segment = canvas.getByRole("button", { name });
      const label = document.createRange();
      label.selectNodeContents(segment);

      await expect(label.getBoundingClientRect().width).toBeLessThanOrEqual(
        contentWidth(segment),
      );
    }
  },
};
