import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CSSProperties } from "react";
import { expect } from "storybook/test";

import { SidebarProvider } from "@/components/ui/sidebar";
import { handlers } from "@/mocks/handlers";

import { LensRail } from "./lens-rail";

const meta: Meta<typeof LensRail> = {
  title: "Portal/LensRail",
  component: LensRail,
  parameters: { layout: "fullscreen", msw: { handlers } },
  decorators: [
    (Story) => (
      <SidebarProvider style={{ "--rail-width": "3.5rem" } as CSSProperties}>
        <Story />
      </SidebarProvider>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof LensRail>;

function horizontalCenter(element: Element): number {
  const box = element.getBoundingClientRect();
  return box.left + box.width / 2;
}

export const TestIconsSitCentredInTheirButtons: Story = {
  tags: ["test"],
  play: async ({ canvas }) => {
    const buttons = await canvas.findAllByRole("button");

    const offCentre = buttons.flatMap((button) => {
      const offset =
        horizontalCenter(button.querySelector("svg")!) -
        horizontalCenter(button);
      return Math.abs(offset) < 0.5 ? [] : [`${button.title} ${offset}px`];
    });

    await expect(offCentre).toEqual([]);
  },
};
