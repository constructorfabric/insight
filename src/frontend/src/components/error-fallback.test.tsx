import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import { ErrorFallback } from "@/components/error-fallback";

const reload = vi.fn();

beforeEach(() => {
  reload.mockClear();
  // jsdom does not implement navigation; swap in an inert reload.
  Object.defineProperty(window, "location", {
    value: { ...window.location, reload },
    writable: true,
    configurable: true,
  });
});

describe("ErrorFallback", () => {
  it("renders an Error's message", () => {
    render(
      <ErrorFallback
        error={new Error("boom")}
        resetErrorBoundary={() => {}}
      />,
    );
    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("boom")).toBeInTheDocument();
  });

  it("renders string errors and falls back for unknown shapes", () => {
    const { unmount } = render(
      <ErrorFallback error="plain failure" resetErrorBoundary={() => {}} />,
    );
    expect(screen.getByText("plain failure")).toBeInTheDocument();
    unmount();

    render(<ErrorFallback error={{ odd: true }} resetErrorBoundary={() => {}} />);
    expect(screen.getByText("Unknown error")).toBeInTheDocument();
  });

  it("wires the try-again and reload actions", async () => {
    const user = userEvent.setup();
    const reset = vi.fn();
    render(<ErrorFallback error={new Error("x")} resetErrorBoundary={reset} />);

    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(reset).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: "Reload" }));
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
