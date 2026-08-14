// @vitest-environment jsdom
/**
 * The review queue, phase 1. What matters: the empty queue is a celebrated
 * goal state, not a blank table; groups come in working order with honest
 * counts and an unknown kind still shows up (the vocabulary is open by
 * contract); selection lives in the URL so an operator can share a link; and
 * the rates strip shows the tenant-wide counts, not the page's.
 */
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AttentionItem, AttentionResponse } from "@/api/identity-client";

vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

const attention = vi.hoisted(() => ({
  q: {
    data: undefined as AttentionResponse | undefined,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  },
}));
vi.mock("@/queries/identity-resolution", () => ({
  useAttention: () => attention.q,
  // The panel under a selection; its own behaviour is account-detail.test's.
  useAccountBinding: () => ({
    data: undefined,
    isLoading: true,
    isError: false,
    error: null,
    refetch: vi.fn(),
  }),
}));

import { portalRouter } from "@/test/portal-router";

import { IdentitiesView } from "./identities-view";

const RATES = { observed: 60, bound: 55, pending: 3, no_evidence: 1, excluded: 1 };

function item(over: Partial<AttentionItem>): AttentionItem {
  return {
    kind: "contested",
    source: "github",
    source_id: "01900000-0000-7000-8000-00000000aa01",
    account_id: "dev-42",
    email: "dev42@example.com",
    username: null,
    candidates: [],
    ...over,
  };
}

beforeEach(() => {
  attention.q.data = undefined;
  attention.q.isLoading = false;
  attention.q.isError = false;
  attention.q.refetch.mockClear();
  portalRouter.reset();
  portalRouter.set({ zone: "manage", item: "identities" });
});

describe("IdentitiesView", () => {
  it("celebrates the empty queue instead of rendering a blank table", () => {
    attention.q.data = { items: [], rates: RATES };
    render(<IdentitiesView />);

    expect(screen.getByText(/everything is resolved/i)).toBeInTheDocument();
    // The rates strip still shows the tenant-wide picture.
    expect(screen.getByText("60")).toBeInTheDocument();
  });

  // An emptied backlog is exactly when a colleague opens the link they were
  // sent, so the celebration must not take the detail panel down with it.
  // `role="status"` is the panel's own loading state (the binding query is
  // mocked pending here) — its presence means AccountDetail mounted.
  it("answers a shared ?acct= link even after the backlog is worked to zero", () => {
    attention.q.data = { items: [], rates: RATES };
    portalRouter.set({
      zone: "manage",
      item: "identities",
      acct: "github:01900000-0000-7000-8000-00000000aa01:dev-42",
    });
    render(<IdentitiesView />);

    expect(screen.getByText(/everything is resolved/i)).toBeInTheDocument();
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("keeps a bare empty queue a plain celebration — no panel, no placeholder", () => {
    attention.q.data = { items: [], rates: RATES };
    render(<IdentitiesView />);

    expect(screen.getByText(/everything is resolved/i)).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(screen.queryByText(/pick an account from the queue/i)).not.toBeInTheDocument();
  });

  it("says so when the server truncated the evidence — partial counts must not read as tenant-wide", () => {
    attention.q.data = { items: [], rates: RATES, truncated: true };
    render(<IdentitiesView />);

    expect(screen.getByText(/cover only part of the observed accounts/i)).toBeInTheDocument();
  });

  // Two different facts: the evidence read hit its ceiling (rates are a
  // prefix) versus the item cap cut the list (rates still whole-tenant).
  it("says the list was cut when only the item cap was hit", () => {
    attention.q.data = { items: [], rates: RATES, items_truncated: true };
    render(<IdentitiesView />);

    expect(screen.getByText(/only the first accounts needing review/i)).toBeInTheDocument();
    expect(
      screen.queryByText(/cover only part of the observed accounts/i),
    ).not.toBeInTheDocument();
  });

  it("does not repeat the item-cap notice when the evidence read was truncated too", () => {
    attention.q.data = {
      items: [],
      rates: RATES,
      truncated: true,
      items_truncated: true,
    };
    render(<IdentitiesView />);

    expect(screen.getByText(/cover only part of the observed accounts/i)).toBeInTheDocument();
    expect(
      screen.queryByText(/only the first accounts needing review/i),
    ).not.toBeInTheDocument();
  });

  it("shows no truncation warning on a complete read, including from an older backend", () => {
    attention.q.data = { items: [], rates: RATES };
    render(<IdentitiesView />);

    expect(
      screen.queryByText(/cover only part of the observed accounts/i),
    ).not.toBeInTheDocument();
  });

  it("groups by kind with honest counts, and an unknown kind still shows", () => {
    attention.q.data = {
      items: [
        item({ account_id: "a1" }),
        item({ account_id: "a2" }),
        item({ kind: "no_evidence", account_id: "bot-1", email: null, username: "bot-1" }),
        item({ kind: "quarantined", account_id: "q-1", email: null, username: null }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    const contested = screen.getByText(/contested/i).closest("[data-slot=card]");
    expect(within(contested as HTMLElement).getByText("2")).toBeInTheDocument();
    expect(screen.getByText(/nothing links the account/i)).toBeInTheDocument();
    // Unknown kind lands in the catch-all group rather than vanishing.
    expect(screen.getByText("q-1")).toBeInTheDocument();
  });

  it("renders candidates as person cells", () => {
    attention.q.data = {
      items: [
        item({
          candidates: [
            {
              person_id: "01900000-0000-7000-8000-000000000001",
              display_name: "Bob Park",
              email: "bob.park@example.com",
            },
          ],
        }),
      ],
      rates: RATES,
    };
    render(<IdentitiesView />);

    expect(screen.getByText("Bob Park")).toBeInTheDocument();
  });

  it("writes the selection into the URL and toggles it off on a second click", async () => {
    attention.q.data = { items: [item({})], rates: RATES };
    render(<IdentitiesView />);

    const row = screen.getByRole("button", { name: /dev42@example\.com/i });
    await userEvent.click(row);
    expect(portalRouter.search.acct).toContain("dev-42");
    expect(row).toHaveAttribute("aria-pressed", "true");

    await userEvent.click(row);
    expect(portalRouter.search.acct).toBeUndefined();
  });

  it("offers a retry on a failed load", async () => {
    attention.q.isError = true;
    render(<IdentitiesView />);

    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(attention.q.refetch).toHaveBeenCalled();
  });
});
