import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import "@/i18n";

import { AnalyticsApiError } from "@/api/analytics-client";

import { QueryEditorDialog } from "./query-editor-dialog";

function setup(overrides: Partial<Parameters<typeof QueryEditorDialog>[0]> = {}) {
  const onSubmit = vi.fn();
  const onOpenChange = vi.fn();
  render(
    <QueryEditorDialog
      open
      onOpenChange={onOpenChange}
      mode="create"
      onSubmit={onSubmit}
      isPending={false}
      error={null}
      {...overrides}
    />
  );
  return { onSubmit, onOpenChange };
}

describe("QueryEditorDialog", () => {
  it("create mode starts blank with Save disabled until name and sql are set", async () => {
    const { onSubmit } = setup();
    expect(screen.getByText("New query")).toBeInTheDocument();

    const save = screen.getByRole("button", { name: "Save" });
    expect(save).toBeDisabled();

    await userEvent.type(screen.getByLabelText("Name"), "My query");
    await userEvent.type(screen.getByLabelText("SQL"), "SELECT 1");
    expect(save).toBeEnabled();

    await userEvent.click(save);
    expect(onSubmit).toHaveBeenCalledWith({
      name: "My query",
      description: "",
      sql: "SELECT 1",
    });
  });

  it("edit mode prefills from initial values", () => {
    setup({
      mode: "edit",
      initial: { name: "Existing", description: "desc", sql: "SELECT 2" },
    });
    expect(screen.getByText("Edit query")).toBeInTheDocument();
    expect(screen.getByLabelText("Name")).toHaveValue("Existing");
    expect(screen.getByLabelText("Description")).toHaveValue("desc");
    expect(screen.getByLabelText("SQL")).toHaveValue("SELECT 2");
  });

  it("surfaces the API field-violation reason", () => {
    setup({
      error: new AnalyticsApiError(400, {
        context: { field_violations: [{ field: "sql", description: "bad sql" }] },
      }),
    });
    expect(screen.getByText("bad sql")).toBeInTheDocument();
  });

  it("Cancel requests close without submitting", async () => {
    const { onOpenChange, onSubmit } = setup();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
