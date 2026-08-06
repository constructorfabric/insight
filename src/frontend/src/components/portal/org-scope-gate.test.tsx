// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { orgScopeGate, type OrgScopeGateArgs } from "./org-scope-gate";

function args(over: Partial<OrgScopeGateArgs>): OrgScopeGateArgs {
  return {
    viewerLoading: false,
    viewerError: false,
    membersLoading: false,
    membersError: false,
    memberCount: 5,
    gridPending: false,
    gridError: false,
    emptyLabel: "No team here.",
    onRetry: vi.fn(),
    ...over,
  };
}

describe("orgScopeGate", () => {
  it("returns null when everything is resolved — the view proceeds", () => {
    expect(orgScopeGate(args({}))).toBeNull();
  });

  it("spins while the viewer identity or roster is loading", () => {
    expect(orgScopeGate(args({ viewerLoading: true }))).not.toBeNull();
    expect(orgScopeGate(args({ membersLoading: true }))).not.toBeNull();
  });

  it("surfaces a backend failure as a retryable error, not an empty team", async () => {
    const onRetry = vi.fn();
    render(<>{orgScopeGate(args({ membersError: true, onRetry }))}</>);
    const retry = screen.getByRole("button", { name: /retry/i });
    await userEvent.click(retry);
    expect(onRetry).toHaveBeenCalledOnce();
  });

  it("shows the empty label when the roster genuinely resolved to nobody", () => {
    render(<>{orgScopeGate(args({ memberCount: 0 }))}</>);
    expect(screen.getByText("No team here.")).toBeInTheDocument();
  });

  it("prioritises the error state over the empty state", () => {
    // A 500 must never masquerade as "this manager has no team".
    render(<>{orgScopeGate(args({ viewerError: true, memberCount: 0 }))}</>);
    expect(screen.queryByText("No team here.")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  it("gates on a pending metric grid after the roster resolves", () => {
    expect(orgScopeGate(args({ gridPending: true }))).not.toBeNull();
  });

  it("gates on a failed metric grid with retry", () => {
    render(<>{orgScopeGate(args({ gridError: true }))}</>);
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });
});
