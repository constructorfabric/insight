import type { Meta, StoryObj } from "@storybook/react-vite";
import type { CSSProperties } from "react";

import { SidebarProvider } from "@/components/ui/sidebar";
import { handlers } from "@/mocks/handlers";
import { expectHorizontallyCentred } from "@/test/storybook/layout";

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

export const Default: Story = {};

export const TestIconsSitCentredInTheirButtons: Story = {
  tags: ["test"],
  play: async ({ canvas }) => {
    const buttons = await canvas.findAllByRole("button");

    for (const button of buttons) {
      expectHorizontallyCentred(
        button,
        button.querySelector("svg")!,
        button.title
      );
    }
  },
};

export const TestLogoSitsCentredInTheRail: Story = {
  tags: ["test"],
  play: async ({ canvas }) => {
    const logo = await canvas.findByRole("img", { name: "Constructor Fabric" });

    expectHorizontallyCentred(canvas.getByTestId("lens-rail"), logo, "Logo");
  },
};
