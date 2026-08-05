import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import i18n from "@/i18n";
import { authStore } from "@/auth/auth-store";
import { AuthGate } from "./auth-gate";
import { makeSession } from "@/test/session";

function renderGate() {
  return render(
    <AuthGate>
      <div>protected</div>
    </AuthGate>,
  );
}

const redirectingText = () => i18n.t("auth.redirecting");

describe("<AuthGate>", () => {
  afterEach(() => {
    authStore.reset();
  });

  it("renders children when authenticated", () => {
    act(() =>
      authStore.setAuthenticated(makeSession()),
    );
    renderGate();
    expect(screen.getByText("protected")).toBeInTheDocument();
  });

  it("shows the redirect overlay instead of children while loading", () => {
    // reset() leaves the store in its initial `loading` status.
    renderGate();
    expect(screen.queryByText("protected")).not.toBeInTheDocument();
    expect(screen.getByText(redirectingText())).toBeInTheDocument();
  });

  it("shows the redirect overlay instead of children when unauthenticated", () => {
    act(() => authStore.setUnauthenticated());
    renderGate();
    expect(screen.queryByText("protected")).not.toBeInTheDocument();
    expect(screen.getByText(redirectingText())).toBeInTheDocument();
  });

  it("swaps to the overlay when a live session drops to unauthenticated", () => {
    act(() =>
      authStore.setAuthenticated(makeSession()),
    );
    renderGate();
    expect(screen.getByText("protected")).toBeInTheDocument();
    act(() => authStore.setUnauthenticated());
    expect(screen.queryByText("protected")).not.toBeInTheDocument();
    expect(screen.getByText(redirectingText())).toBeInTheDocument();
  });
});
