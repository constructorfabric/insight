import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect } from "storybook/test";
import type { CSSProperties } from "react";

import { SidebarProvider } from "@gears-frontx/ui-kit";
import { setPortalRailLabels } from "@/lib/portal/portal-store";
import { handlers } from "@/mocks/handlers";
import { expectHorizontallyCentred } from "@/test/storybook/layout";

import { LensRail } from "./lens-rail";

const meta: Meta<typeof LensRail> = {
  title: "Portal/LensRail",
  component: LensRail,
  parameters: { layout: "fullscreen", msw: { handlers } },
  decorators: [
    (Story) => (
      <SidebarProvider
        style={
          {
            "--rail-width": "calc(var(--sidebar-width-icon) + 1px)",
          } as CSSProperties
        }
      >
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
        button.textContent ?? ""
      );
    }
  },
};

export const TestLogoSitsCentredInTheRailHeader: Story = {
  tags: ["test"],
  play: async ({ canvas }) => {
    const logo = await canvas.findByRole("img", { name: "Constructor Fabric" });

    expectHorizontallyCentred(logo.parentElement!, logo, "Logo");
  },
};

export const NamesShown: Story = {
  beforeEach: () => {
    setPortalRailLabels(true);
    return () => setPortalRailLabels(false);
  },
  decorators: [
    (Story) => (
      <div style={{ "--rail-width": "12rem" } as CSSProperties}>
        <Story />
      </div>
    ),
  ],
};

export const TestTheToggleStaysIconOnlyWithNamesShown: Story = {
  ...NamesShown,
  tags: ["test"],
  play: async ({ canvas }) => {
    const toggle = await canvas.findByRole("button", {
      name: "Collapse navigation",
    });

    expect(toggle.textContent).toBe("");
  },
};
