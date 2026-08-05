import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import { AnalyticsApiError } from "@/api/analytics-client";
import type { SavedQuery } from "@/api/saved-queries-client";

type QueryState = { data?: SavedQuery; isPending: boolean; isError: boolean };
type RunState = {
  isPending: boolean;
  isError: boolean;
  isSuccess: boolean;
  data?: { rows: unknown[] };
  error?: unknown;
};

let queryState: QueryState;
let runState: RunState;
const runMutate = vi.fn();

vi.mock("@/queries/saved-queries", () => ({
  useSavedQuery: () => queryState,
  useRunSavedQuery: () => ({ ...runState, mutate: runMutate }),
}));

vi.mock("./query-results", () => ({
  QueryResults: ({ rows }: { rows: unknown[] }) => (
    <div>results:{rows.length}</div>
  ),
}));

import { QueryDetail } from "./query-detail";

const QUERY: SavedQuery = {
  id: "q-1",
  insight_tenant_id: "t-1",
  name: "Commits by tool",
  description: "desc",
  sql: "SELECT tool, commits FROM x",
  created_at: "2026-07-01T00:00:00Z",
  updated_at: "2026-07-01T00:00:00Z",
};

function renderDetail(over: Partial<Record<string, unknown>> = {}) {
  const onEdit = vi.fn();
  const onDelete = vi.fn();
  render(
    <QueryDetail id="q-1" onEdit={onEdit} onDelete={onDelete} {...over} />
  );
  return { onEdit, onDelete };
}

beforeEach(() => {
  queryState = { data: QUERY, isPending: false, isError: false };
  runState = { isPending: false, isError: false, isSuccess: false };
  runMutate.mockReset();
});

describe("QueryDetail", () => {
  it("shows a spinner while the query is loading", () => {
    queryState = { data: undefined, isPending: true, isError: false };
    renderDetail();
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("shows a load error", () => {
    queryState = { data: undefined, isPending: false, isError: true };
    renderDetail();
    expect(screen.getByText("Failed to load this query")).toBeInTheDocument();
  });

  it("renders the query name and sql", () => {
    renderDetail();
    expect(screen.getByText("Commits by tool")).toBeInTheDocument();
    expect(screen.getByText("SELECT tool, commits FROM x")).toBeInTheDocument();
  });

  it("fires onEdit and onDelete with the id", async () => {
    const { onEdit, onDelete } = renderDetail();
    await userEvent.click(screen.getByRole("button", { name: /edit/i }));
    await userEvent.click(screen.getByRole("button", { name: /delete/i }));
    expect(onEdit).toHaveBeenCalledWith("q-1");
    expect(onDelete).toHaveBeenCalledWith("q-1");
  });

  it("runs with a trimmed period, or an empty body when blank", async () => {
    renderDetail();
    const run = screen.getByRole("button", { name: /run/i });

    await userEvent.click(run);
    expect(runMutate).toHaveBeenLastCalledWith({});

    await userEvent.type(screen.getByLabelText("Period"), "  2026-01  ");
    await userEvent.click(run);
    expect(runMutate).toHaveBeenLastCalledWith({ period: "2026-01" });
  });

  it("renders results on run success", () => {
    runState = {
      isPending: false,
      isError: false,
      isSuccess: true,
      data: { rows: [{}, {}] },
    };
    renderDetail();
    expect(screen.getByText("results:2")).toBeInTheDocument();
  });

  it("surfaces a run error reason", () => {
    runState = {
      isPending: false,
      isError: true,
      isSuccess: false,
      error: new AnalyticsApiError(400, {
        context: {
          field_violations: [
            { field: "period", description: "period is required" },
          ],
        },
      }),
    };
    renderDetail();
    expect(screen.getByText("period is required")).toBeInTheDocument();
  });
});
