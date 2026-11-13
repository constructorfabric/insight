import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, within } from "storybook/test";

import { BASE, type FolderList } from "@/api/custom-client";
import {
  Sidebar,
  SidebarContent,
  SidebarProvider,
} from "@/components/ui/sidebar";

import { CustomNav } from "./custom-nav";

const FOLDERS: FolderList = {
  folders: [
    {
      id: "f-1",
      name: "Team reviews",
      owner_person_id: "p-1",
      access: "owner",
      dashboards: 128,
    },
    {
      id: "f-3",
      name: "Quarterly",
      owner_person_id: "p-1",
      access: "owner",
      dashboards: 3,
    },
    {
      id: "f-2",
      name: "Read only",
      owner_person_id: "p-2",
      access: "viewer",
      dashboards: 12,
    },
  ],
  unfiled: 0,
};

const meta: Meta<typeof CustomNav> = {
  title: "Portal/CustomNav",
  component: CustomNav,
  parameters: {
    layout: "fullscreen",
    msw: {
      handlers: [
        http.get(`${BASE}/folders`, () => HttpResponse.json(FOLDERS)),
        http.get(`${BASE}/dashboards`, () =>
          HttpResponse.json({ items: [], total: 0 })
        ),
        http.get(`${BASE}/pins`, () => HttpResponse.json({ pins: [] })),
      ],
    },
  },
  decorators: [
    (Story) => (
      <SidebarProvider>
        <Sidebar collapsible="none">
          <SidebarContent>
            <Story />
          </SidebarContent>
        </Sidebar>
      </SidebarProvider>
    ),
  ],
};
export default meta;

type Story = StoryObj<typeof CustomNav>;

export const Default: Story = {};

export const TestFolderCountSitsFlushRightAfterItsMoreButton: Story = {
  tags: ["test"],
  play: async ({ canvas }) => {
    const browseRow = (
      await canvas.findByRole("link", { name: "All dashboards" })
    ).closest("li")!;
    const browseCount = await within(browseRow).findByText("0");
    const more = await canvas.findByRole("button", {
      name: "More for Team reviews",
    });
    const widestCount = canvas.getByText("128").getBoundingClientRect();

    await expect(widestCount.right).toBe(
      browseCount.getBoundingClientRect().right
    );
    await expect(more.getBoundingClientRect().right).toBeLessThanOrEqual(
      widestCount.left
    );
  },
};
