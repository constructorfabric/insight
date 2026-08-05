import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { SavedQuery, SavedQuerySummary } from "@/api/saved-queries-client";

let listState: {
  data?: SavedQuerySummary[];
  isPending: boolean;
  isError: boolean;
};
let savedQueryState: { data?: SavedQuery; isError: boolean };
let deleteState: { isError: boolean };

// Mutations invoke their success callback synchronously so the screen's
// onSuccess wiring (select/close/reset) is exercised.
const createMutate = vi.fn(
  (_body: unknown, opts?: { onSuccess?: (q: { id: string }) => void }) =>
    opts?.onSuccess?.({ id: "new-id" })
);
const updateMutate = vi.fn(
  (_body: unknown, opts?: { onSuccess?: () => void }) => opts?.onSuccess?.()
);
const deleteMutate = vi.fn(
  (_id: string, opts?: { onSuccess?: () => void }) => opts?.onSuccess?.()
);

vi.mock("@/queries/saved-queries", () => ({
  useSavedQueries: () => listState,
  useCreateSavedQuery: () => ({
    mutate: createMutate,
    isPending: false,
    error: null,
  }),
  useDeleteSavedQuery: () => ({ mutate: deleteMutate, ...deleteState }),
  useUpdateSavedQuery: () => ({
    mutate: updateMutate,
    isPending: false,
    error: null,
  }),
  useSavedQuery: () => savedQueryState,
}));

vi.mock("@/components/ui/sidebar", () => ({ SidebarTrigger: () => null }));

// Stubs that actually invoke their callback props, so the screen's delete /
// edit-open / create-success / update branches run in tests.
vi.mock("@/components/widgets/query-console/query-detail", () => ({
  QueryDetail: ({
    id,
    onEdit,
    onDelete,
  }: {
    id: string;
    onEdit: (id: string) => void;
    onDelete: (id: string) => void;
  }) => (
    <div>
      detail:{id}
      <button onClick={() => onEdit(id)}>stub-edit</button>
      <button onClick={() => onDelete(id)}>stub-delete</button>
    </div>
  ),
}));

vi.mock("@/components/widgets/query-console/query-editor-dialog", () => ({
  QueryEditorDialog: ({
    mode,
    onSubmit,
    onOpenChange,
  }: {
    mode: string;
    onSubmit: (d: { name: string; description: string; sql: string }) => void;
    onOpenChange: (open: boolean) => void;
  }) => (
    <div>
      editor:{mode}
      <button
        onClick={() =>
          onSubmit({ name: "n", description: "", sql: "select 1" })
        }
      >
        stub-submit
      </button>
      <button onClick={() => onOpenChange(false)}>stub-close</button>
    </div>
  ),
}));

import { QueryConsoleScreen } from "./query-console";

function summary(over: Partial<SavedQuerySummary> = {}): SavedQuerySummary {
  return { id: "q-42", name: "Commits by tool", description: null, ...over };
}

const SAVED: SavedQuery = {
  id: "q-42",
  insight_tenant_id: "t-1",
  name: "Commits by tool",
  description: null,
  sql: "SELECT 1",
  created_at: "2026-07-01T00:00:00Z",
  updated_at: "2026-07-01T00:00:00Z",
};

beforeEach(() => {
  listState = { data: [summary()], isPending: false, isError: false };
  savedQueryState = { data: SAVED, isError: false };
  deleteState = { isError: false };
  createMutate.mockClear();
  updateMutate.mockClear();
  deleteMutate.mockClear();
});

describe("QueryConsoleScreen", () => {
  it("shows a spinner while the list is pending", () => {
    listState = { data: undefined, isPending: true, isError: false };
    render(<QueryConsoleScreen />);
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("shows an error alert when the list fails", () => {
    listState = { data: undefined, isPending: false, isError: true };
    render(<QueryConsoleScreen />);
    expect(
      screen.getByText("Failed to load saved queries")
    ).toBeInTheDocument();
  });

  it("shows an empty-list hint and no selection when there are no queries", () => {
    listState = { data: [], isPending: false, isError: false };
    render(<QueryConsoleScreen />);
    expect(
      screen.getByText("No saved queries yet. Create one to get started.")
    ).toBeInTheDocument();
    expect(screen.getByText("No query selected")).toBeInTheDocument();
  });

  it("selects a query and renders its detail pane", async () => {
    render(<QueryConsoleScreen />);
    await userEvent.click(screen.getByText("Commits by tool"));
    expect(screen.getByText("detail:q-42")).toBeInTheDocument();
  });

  it("creates a query, then selects it and closes the dialog", async () => {
    render(<QueryConsoleScreen />);
    await userEvent.click(screen.getByRole("button", { name: /new query/i }));
    expect(screen.getByText("editor:create")).toBeInTheDocument();

    await userEvent.click(screen.getByText("stub-submit"));
    expect(createMutate).toHaveBeenCalledWith(
      { name: "n", description: null, sql: "select 1" },
      expect.any(Object)
    );
    expect(screen.getByText("detail:new-id")).toBeInTheDocument();
    expect(screen.queryByText("editor:create")).not.toBeInTheDocument();
  });

  it("deletes the selected query and clears the selection", async () => {
    render(<QueryConsoleScreen />);
    await userEvent.click(screen.getByText("Commits by tool"));
    await userEvent.click(screen.getByText("stub-delete"));
    expect(deleteMutate).toHaveBeenCalledWith("q-42", expect.any(Object));
    expect(screen.getByText("No query selected")).toBeInTheDocument();
  });

  it("opens the edit dialog and submits an update", async () => {
    render(<QueryConsoleScreen />);
    await userEvent.click(screen.getByText("Commits by tool"));
    await userEvent.click(screen.getByText("stub-edit"));
    expect(screen.getByText("editor:edit")).toBeInTheDocument();

    await userEvent.click(screen.getByText("stub-submit"));
    expect(updateMutate).toHaveBeenCalledWith(
      { name: "n", description: null, sql: "select 1" },
      expect.any(Object)
    );
  });

  it("shows a spinner in the edit dialog while the query loads", async () => {
    savedQueryState = { data: undefined, isError: false };
    render(<QueryConsoleScreen />);
    await userEvent.click(screen.getByText("Commits by tool"));
    await userEvent.click(screen.getByText("stub-edit"));
    expect(screen.getByRole("status")).toBeInTheDocument();
  });

  it("shows an error in the edit dialog when the query fails to load", async () => {
    savedQueryState = { data: undefined, isError: true };
    render(<QueryConsoleScreen />);
    await userEvent.click(screen.getByText("Commits by tool"));
    await userEvent.click(screen.getByText("stub-edit"));
    expect(screen.getByText("Failed to load this query")).toBeInTheDocument();
  });

  it("surfaces a delete failure", () => {
    deleteState = { isError: true };
    render(<QueryConsoleScreen />);
    expect(
      screen.getByText("Could not delete the query. Try again.")
    ).toBeInTheDocument();
  });
});
