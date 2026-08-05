import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import { MockBanner } from "@/components/mock-banner";

// `isVisible()` also requires import.meta.env.DEV, which vitest sets to true,
// so the mock flags below are the only knobs the tests need to turn.
describe("MockBanner", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  it("renders nothing when mocks are not enabled", () => {
    vi.stubEnv("VITE_ENABLE_MOCKS", "false");
    const { container } = render(<MockBanner />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows the synthetic-data warning when mocks are enabled", () => {
    vi.stubEnv("VITE_ENABLE_MOCKS", "true");
    render(<MockBanner />);
    expect(screen.getByText("Synthetic data")).toBeInTheDocument();
  });

  it("stays hidden when the banner is explicitly suppressed", () => {
    vi.stubEnv("VITE_ENABLE_MOCKS", "true");
    vi.stubEnv("VITE_HIDE_MOCK_BANNER", "true");
    const { container } = render(<MockBanner />);
    expect(container).toBeEmptyDOMElement();
  });
});
