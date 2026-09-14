/**
 * The admin role control in the person window, driven against a stubbed
 * identity service rather than stubbed hooks.
 *
 * The handlers below hold the assignment, so a grant changes what the next read
 * returns and the badge follows — the client, the query layer and the component
 * are all exercised over HTTP. The jsdom suite in `person-dialog.test.tsx`
 * covers the same states with the hooks replaced; this is the round trip.
 *
 * See docs/testing/storybook-component-tests.md.
 */
import type { Meta, StoryObj } from "@storybook/react-vite";
import { http, HttpResponse } from "msw";
import { expect, userEvent, within } from "storybook/test";

import { authStore } from "@/auth/auth-store";
import { PersonDialog } from "@/components/portal/person-dialog";
import { makeSession } from "@/test/session";

const PERSON = "019e27bc-dec0-7626-81a9-c5524662a6a9";
const ADMIN_ROLE_ID = "a4d11000-0000-4000-8000-000000000001";

function assignment() {
  return {
    person_role_id: "019e27bc-0000-7000-8000-000000000001",
    insight_tenant_id: "3f1d8f4e-6c2a-4a9b-91d7-8e5c0b2a7f36",
    person_id: PERSON,
    role_id: ADMIN_ROLE_ID,
    valid_from: "2026-09-14T10:28:18Z",
    valid_to: null,
    author_person_id: "019e27bc-dec0-7626-81a9-c5524662a6aa",
    reason: null,
    created_at: "2026-09-14T10:28:18Z",
  };
}

function identity({
  start = false,
  viewer = "admin",
  read = "ok",
  revoke = "ok",
}: {
  start?: boolean;
  viewer?: "admin" | "none";
  read?: "ok" | "error";
  revoke?: "ok" | "last-admin";
} = {}) {
  // The stub's own state: a grant has to change what the next read answers, or
  // the badge would only ever reflect the fixture.
  let held = start;

  return [
    http.get("*/api/identity/v1/me", () =>
      HttpResponse.json({
        person_id: "019e27bc-dec0-7626-81a9-c5524662a6aa",
        insight_tenant_id: "3f1d8f4e-6c2a-4a9b-91d7-8e5c0b2a7f36",
        roles:
          viewer === "admin" ? [{ role_id: ADMIN_ROLE_ID, name: "admin" }] : [],
      })
    ),
    http.get("*/api/identity/v1/person-roles", () => {
      if (viewer !== "admin" || read === "error") {
        return HttpResponse.json(
          { context: { reason: "admin role required for this operation" } },
          { status: 403 }
        );
      }
      return HttpResponse.json({
        items: held ? [assignment()] : [],
        next_cursor: null,
      });
    }),
    http.post("*/api/identity/v1/person-roles", () => {
      held = true;
      return HttpResponse.json(assignment(), { status: 201 });
    }),
    http.delete("*/api/identity/v1/person-roles/:id", () => {
      if (revoke === "last-admin") {
        return HttpResponse.json(
          { context: { reason: "last_admin_protected" } },
          { status: 409 }
        );
      }
      held = false;
      return new HttpResponse(null, { status: 204 });
    }),
    http.get("*/api/identity/v1/resolution/persons/*/accounts", () =>
      HttpResponse.json({ person_id: PERSON, accounts: [] })
    ),
  ];
}

/** The window renders in a portal: the story canvas holds only the inert
 *  backdrop, so every query runs against the document. */
const body = () => within(document.body);

const meta: Meta<typeof PersonDialog> = {
  title: "Portal/PersonDialog",
  component: PersonDialog,
  args: {
    personId: PERSON,
    card: { person_id: PERSON, display_name: "Ada Lovelace" },
    onClose: () => {},
  },
  parameters: {
    layout: "fullscreen",
    msw: { handlers: identity() },
  },
  // The preview resets `authStore` before every story, and `useMe` is keyed on
  // the session scope — without one the role read never fires and the control
  // renders as "not an admin".
  beforeEach: () => {
    authStore.setAuthenticated(makeSession());
  },
};
export default meta;

type Story = StoryObj<typeof PersonDialog>;

/** The subject holds nothing yet — the window offers the grant. */
export const NotAdmin: Story = {};

/** The subject holds the role — badge, and the withdrawing verb. */
export const IsAdmin: Story = {
  parameters: { msw: { handlers: identity({ start: true }) } },
};

/** The viewer holds admin but the read failed: "unknown", and no verb. */
export const RolesUnreadable: Story = {
  parameters: { msw: { handlers: identity({ read: "error" }) } },
};

/** A viewer who is not an admin sees no role control at all. */
export const ViewerNotAdmin: Story = {
  parameters: { msw: { handlers: identity({ viewer: "none" }) } },
};

export const TestGrant: Story = {
  tags: ["test"],
  play: async () => {
    const canvas = body();

    await userEvent.click(
      await canvas.findByRole("button", { name: "Make admin" })
    );

    await expect(await canvas.findByText("Admin")).toBeInTheDocument();
    await expect(
      await canvas.findByRole("button", { name: "Remove admin" })
    ).toBeInTheDocument();
  },
};

export const TestRevoke: Story = {
  tags: ["test"],
  parameters: { msw: { handlers: identity({ start: true }) } },
  play: async () => {
    const canvas = body();

    await userEvent.click(
      await canvas.findByRole("button", { name: "Remove admin" })
    );

    await expect(
      await canvas.findByRole("button", { name: "Make admin" })
    ).toBeInTheDocument();
    await expect(canvas.queryByText("Admin")).not.toBeInTheDocument();
  },
};

export const TestRefusedReadOffersNoVerb: Story = {
  tags: ["test"],
  parameters: { msw: { handlers: identity({ read: "error" }) } },
  play: async () => {
    const canvas = body();

    // Anchor on the window first: without it every queryBy below passes on an
    // empty document, which is how this case passed while querying the canvas.
    await canvas.findByRole("dialog");
    await expect(
      await canvas.findByText("Admin status could not be read.")
    ).toBeInTheDocument();

    await expect(
      canvas.queryByRole("button", { name: "Make admin" })
    ).not.toBeInTheDocument();
    await expect(
      canvas.queryByRole("button", { name: "Remove admin" })
    ).not.toBeInTheDocument();
  },
};

export const TestLastAdminRefusalIsNamed: Story = {
  tags: ["test"],
  parameters: {
    msw: { handlers: identity({ start: true, revoke: "last-admin" }) },
  },
  play: async () => {
    const canvas = body();

    await userEvent.click(
      await canvas.findByRole("button", { name: "Remove admin" })
    );

    await expect(
      await canvas.findByText("The tenant's last admin cannot be removed.")
    ).toBeInTheDocument();
    await expect(await canvas.findByText("Admin")).toBeInTheDocument();
  },
};

export const TestNonAdminViewerSeesNothing: Story = {
  tags: ["test"],
  parameters: { msw: { handlers: identity({ viewer: "none", start: true }) } },
  play: async () => {
    const canvas = body();

    // The viewer gate short-circuits ahead of the read, so this is silence
    // rather than the "unknown" wording — the subject holds the role, and a
    // viewer who may not manage roles is told nothing about it either way.
    await canvas.findByRole("dialog");
    await expect(
      canvas.queryByText("Admin status could not be read.")
    ).not.toBeInTheDocument();
    await expect(canvas.queryByText("Admin")).not.toBeInTheDocument();
    await expect(
      canvas.queryByRole("button", { name: "Make admin" })
    ).not.toBeInTheDocument();
  },
};
