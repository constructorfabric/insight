import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent } from "storybook/test";

import { CustomPageShell } from "@/components/custom/custom-page-shell";
import { DefinitionList } from "@/components/custom/definition-list";
import { NewLink } from "@/components/custom/editor/edit-link";
import {
  CARD_PX_WIDE,
  expectNothingOverlaps,
} from "@/test/storybook/layout";

function Catalogue() {
  return (
    <CustomPageShell chat={<p>assistant</p>}>
      <DefinitionList
        title="Datasets"
        blurb="Every declared dataset. A metric reads one of these, and records are sent into them."
        names={[]}
        isLoading={false}
        isError={false}
        onRetry={() => {}}
        create={<NewLink kind="datasets" noun="dataset" />}
        emptyLabel="No datasets yet. Declare one to send records into it."
        renderRow={() => null}
      />
    </CustomPageShell>
  );
}

const meta: Meta<typeof Catalogue> = {
  title: "Custom/DefinitionList",
  component: Catalogue,
  beforeEach: () => {
    window.localStorage.removeItem("insight.custom.assistant-hidden");
  },
};
export default meta;

type Story = StoryObj<typeof Catalogue>;

export const Default: Story = {};

export const TestHiddenAssistantToggleClearsTheCreateButton: Story = {
  decorators: [
    (Story) => (
      <div style={{ width: CARD_PX_WIDE }}>
        <Story />
      </div>
    ),
  ],
  tags: ["test"],
  play: async ({ canvas }) => {
    await userEvent.click(
      canvas.getByRole("button", { name: "Hide the assistant" })
    );

    const create = canvas.getByRole("button", { name: "New dataset" });
    const toggle = canvas.getByRole("button", { name: "Show the assistant" });

    await expect(create).toBeVisible();
    expectNothingOverlaps(create, [toggle], () => "The assistant toggle");
  },
};
