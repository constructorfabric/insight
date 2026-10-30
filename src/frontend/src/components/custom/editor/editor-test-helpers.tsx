import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { screen } from "@testing-library/react";
import type userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";
import { datasetNames } from "@/test/catalogue";

export function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

/** Every catalogue answers empty, every dataset declares nothing, and no definition is found, unless a test says otherwise. */
export function mockCatalogues(): void {
  const none = { items: [], total: 0 };
  vi.mocked(customClient.fetchMetrics).mockResolvedValue(none);
  vi.mocked(customClient.fetchWidgets).mockResolvedValue(none);
  vi.mocked(customClient.fetchDashboards).mockResolvedValue(none);
  vi.mocked(customClient.fetchDatasetNames).mockResolvedValue(datasetNames([]));
  vi.mocked(customClient.fetchMetric).mockRejectedValue(
    new CustomApiError(404, null)
  );
  vi.mocked(customClient.fetchWidget).mockRejectedValue(
    new CustomApiError(404, null)
  );
  vi.mocked(customClient.fetchDataset).mockImplementation(async (name) => ({
    name,
    declaration: { title: name, fields: [] },
  }));
}

/** A 400 as the service sends it: places in the document, each with a reason. */
export function invalid(
  violations: { field?: string; description: string }[]
): CustomApiError {
  return new CustomApiError(400, {
    detail: "Request validation failed",
    context: {
      field_violations: violations.map((one) => ({
        ...one,
        reason: "INVALID",
      })),
    },
  });
}

/** Replaces the document's text with what is given, as a person pasting it would. */
export async function writeText(
  user: ReturnType<typeof userEvent.setup>,
  text: string
): Promise<HTMLTextAreaElement> {
  const area = screen.getByLabelText(/as text/i) as HTMLTextAreaElement;
  await user.clear(area);
  await user.paste(text);
  return area;
}
