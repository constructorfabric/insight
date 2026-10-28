vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, fetchMetrics: vi.fn(), fetchMetric: vi.fn() };
});

import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import type { StoredMetric } from "@/api/custom-types";
import { wrapper } from "@/test/alerts";

import { DefinitionPicker } from "./definition-picker";

const LINES_PER_DAY: StoredMetric = {
  name: "Lines per day",
  definition: { table: "gold.lines", fields: [] },
};

function renderChosen() {
  render(
    <DefinitionPicker
      kind="metrics"
      id="metric"
      value="m-1"
      onChange={vi.fn()}
    />,
    { wrapper }
  );
  return screen.getByRole("combobox");
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(customClient.fetchMetrics).mockResolvedValue({
    items: [],
    total: 0,
  });
});

describe("DefinitionPicker, for a chosen definition the list has not read", () => {
  it("shows nothing while the name is read, never the id", async () => {
    vi.mocked(customClient.fetchMetric).mockReturnValue(new Promise(() => {}));

    const input = renderChosen();

    await waitFor(() =>
      expect(customClient.fetchMetric).toHaveBeenCalledWith("m-1")
    );
    expect(input).toHaveValue("");
  });

  it("shows the name once it is read", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(LINES_PER_DAY);

    const input = renderChosen();

    await waitFor(() => expect(input).toHaveValue("Lines per day"));
  });

  it("falls back to the id only when the name cannot be read", async () => {
    vi.mocked(customClient.fetchMetric).mockRejectedValue(
      new CustomApiError(404, null)
    );

    const input = renderChosen();

    await waitFor(() => expect(input).toHaveValue("m-1"));
  });
});
