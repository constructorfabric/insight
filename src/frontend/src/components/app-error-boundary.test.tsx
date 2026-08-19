import * as Sentry from "@sentry/react";
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import "@/i18n";

import { AppErrorBoundary } from "@/components/app-error-boundary";

vi.mock("@sentry/react", () => ({ captureReactException: vi.fn() }));

function BrokenChild(): React.ReactElement {
  throw new Error("render failed");
}

beforeEach(() => {
  // React logs every caught error through console.error.
  vi.spyOn(console, "error").mockImplementation(() => {});
});

afterEach(() => {
  vi.restoreAllMocks();
});

it("reports a render error with its component stack", () => {
  render(
    <AppErrorBoundary>
      <BrokenChild />
    </AppErrorBoundary>
  );

  expect(screen.getByText("render failed")).toBeInTheDocument();
  const [, info] = vi.mocked(Sentry.captureReactException).mock.calls[0];
  expect(info.componentStack).toContain("BrokenChild");
});
