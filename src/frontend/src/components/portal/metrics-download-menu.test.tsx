import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import { MetricsDownloadMenu } from "./metrics-download-menu";

const mocks = vi.hoisted(() => ({
  toastAdd: vi.fn(),
  downloadMatrixXlsx: vi.fn<() => Promise<void>>(),
}));

vi.mock("@gears-frontx/ui-kit", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@gears-frontx/ui-kit")>()),
  toast: { add: mocks.toastAdd },
}));

vi.mock("@/lib/export/matrix", () => ({
  downloadMatrixCsv: vi.fn(),
  downloadMatrixXlsx: mocks.downloadMatrixXlsx,
}));

vi.mock("@/telemetry", () => ({ recordUsageEvent: vi.fn() }));

const VIEW = {
  people: [{ entityId: "p-jane", displayName: "Jane Doe" }],
  keys: [],
  byKey: new Map(),
  roles: new Map(),
  range: { from: "2026-07-01", to: "2026-07-31" },
};

describe("MetricsDownloadMenu", () => {
  beforeEach(() => {
    mocks.toastAdd.mockReset();
    mocks.downloadMatrixXlsx.mockReset();
  });

  it("tells the user when the Excel file cannot be made, and lets them try again", async () => {
    mocks.downloadMatrixXlsx.mockRejectedValue(new Error("chunk failed to load"));
    const user = userEvent.setup();
    render(<MetricsDownloadMenu view={VIEW} disabled={false} />);

    await user.click(screen.getByRole("button", { name: "Download" }));
    await user.click(await screen.findByRole("menuitem", { name: "Excel (.xlsx)" }));

    await waitFor(() =>
      expect(mocks.toastAdd).toHaveBeenCalledWith(
        expect.objectContaining({ type: "error", title: "The download failed. Try again." }),
      ),
    );
    expect(screen.getByRole("button", { name: "Download" })).toBeEnabled();
  });
});
