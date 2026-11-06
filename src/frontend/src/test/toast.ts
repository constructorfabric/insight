import type { Mock } from "vitest";

type Shown = { type?: string; title?: unknown; description?: unknown; timeout?: number };

/** The toasts a mocked `toast.add` was asked to show, of one type. */
export function toastsOf(add: Mock, type: "success" | "error"): Shown[] {
  return add.mock.calls
    .map(([toast]) => toast as Shown)
    .filter((toast) => toast.type === type);
}
