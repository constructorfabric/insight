import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { screen } from "@testing-library/react";
import type userEvent from "@testing-library/user-event";
import { createElement, type ReactNode } from "react";
import { vi } from "vitest";

import * as customClient from "@/api/custom-client";
import { CustomApiError } from "@/api/custom-client";

export function wrapper({ children }: { children: ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

/** Every catalogue answers empty, and every dataset declares nothing, unless a test says otherwise. */
export function mockCatalogues(): void {
  const none = { names: [], total: 0 };
  vi.mocked(customClient.fetchMetricNames).mockResolvedValue(none);
  vi.mocked(customClient.fetchWidgetNames).mockResolvedValue(none);
  vi.mocked(customClient.fetchDashboardNames).mockResolvedValue(none);
  vi.mocked(customClient.fetchDatasetNames).mockResolvedValue(none);
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

/** What an input offers to pick from, in the order it offers it. */
export function offeredBy(input: HTMLElement): string[] {
  const listId = input.getAttribute("list");
  const list = listId ? document.getElementById(listId) : null;
  if (!list) throw new Error("input offers nothing");
  return [...list.querySelectorAll("option")].map((option) => option.value);
}

/** The row a control sits in: its label, hint and whatever was said about it. */
export function rowOf(control: HTMLElement): HTMLElement {
  const row = control.closest("div.flex.flex-col");
  if (!(row instanceof HTMLElement)) throw new Error("control has no row");
  return row;
}

export async function showText(
  user: ReturnType<typeof userEvent.setup>
): Promise<HTMLTextAreaElement> {
  await user.click(screen.getByRole("button", { name: "Text" }));
  return screen.getByLabelText(/as text/i) as HTMLTextAreaElement;
}
