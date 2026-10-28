vi.mock("@/api/custom-client", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/api/custom-client")>();
  return { ...actual, fetchMetric: vi.fn() };
});

import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import type { StoredMetric } from "@/api/custom-types";
import { wrapper } from "@/test/alerts";

import { MetricName } from "./metric-name";

const LINES_PER_DAY: StoredMetric = {
  name: "Lines per day",
  definition: { table: "gold.lines", fields: [] },
};

beforeEach(() => {
  vi.resetAllMocks();
});

describe("MetricName", () => {
  it("shows nothing while the name is read, never the id", () => {
    vi.mocked(customClient.fetchMetric).mockReturnValue(new Promise(() => {}));

    const { container } = render(<MetricName id="m-1" />, { wrapper });

    expect(container).toHaveTextContent("");
    expect(customClient.fetchMetric).toHaveBeenCalledWith("m-1");
  });

  it("shows the metric's name once it is read", async () => {
    vi.mocked(customClient.fetchMetric).mockResolvedValue(LINES_PER_DAY);

    render(<MetricName id="m-1" />, { wrapper });

    expect(await screen.findByText("Lines per day")).toBeInTheDocument();
    expect(screen.queryByText("m-1")).toBeNull();
  });

  it("falls back to the id only when the name cannot be read", async () => {
    vi.mocked(customClient.fetchMetric).mockRejectedValue(
      new CustomApiError(404, null)
    );

    const { container } = render(<MetricName id="m-1" />, { wrapper });

    await waitFor(() => expect(container).toHaveTextContent("m-1"));
  });
});
