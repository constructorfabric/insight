import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { blankLeaf, type ColumnKind } from "@/lib/alerts/condition";
import type { FieldErrors } from "@/lib/alerts/draft";

import { ConditionBuilder } from "./condition-builder";

const KINDS = new Map<string, ColumnKind>([
  ["team", "text"],
  ["open", "number"],
  ["stale", "number"],
  ["opened_at", "date"],
]);
const COLUMNS = [...KINDS.keys()];

function Harness({
  initial,
  manyRows = true,
  errors = {},
  onChange,
}: {
  initial: unknown;
  manyRows?: boolean;
  errors?: FieldErrors;
  onChange: (condition: unknown) => void;
}) {
  const [condition, setCondition] = useState(initial);

  return (
    <ConditionBuilder
      condition={condition}
      columns={COLUMNS}
      kinds={KINDS}
      manyRows={manyRows}
      errors={errors}
      onChange={(next) => {
        setCondition(next);
        onChange(next);
      }}
    />
  );
}

function setUp(
  initial: unknown,
  props: Partial<Parameters<typeof Harness>[0]> = {}
) {
  const changed = vi.fn();
  render(<Harness initial={initial} onChange={changed} {...props} />);
  return () => changed.mock.lastCall?.[0] as unknown;
}

function leaf(number: number) {
  return within(screen.getByRole("group", { name: `Condition ${number}` }));
}

function optionsOf(select: HTMLElement): string[] {
  return within(select)
    .getAllByRole("option")
    .filter((option) => !(option as HTMLOptionElement).disabled)
    .map((option) => option.textContent ?? "");
}

const BLANK = { rows: "one", where: blankLeaf() };

describe("ConditionBuilder", () => {
  it("builds a nested condition with a quantifier through its controls", async () => {
    const latest = setUp(BLANK);

    await userEvent.selectOptions(screen.getByLabelText("Rows"), "any");
    await userEvent.selectOptions(leaf(1).getByLabelText("Column"), "open");
    await userEvent.type(leaf(1).getByLabelText("Value"), "10");
    await userEvent.click(screen.getByRole("button", { name: "Add group" }));
    const group = within(screen.getByRole("group", { name: "Group 1" }));
    await userEvent.selectOptions(group.getByLabelText("Combine"), "any");
    await userEvent.selectOptions(leaf(2).getByLabelText("Column"), "team");
    await userEvent.type(leaf(2).getByLabelText("Value"), "core");
    await userEvent.click(group.getByRole("button", { name: "Add condition" }));
    await userEvent.selectOptions(leaf(3).getByLabelText("Column"), "stale");
    await userEvent.type(leaf(3).getByLabelText("Value"), "3");

    expect(latest()).toEqual({
      rows: "any",
      where: {
        all: [
          { column: "open", op: ">", value: 10 },
          {
            any: [
              { column: "team", op: "==", value: "core" },
              { column: "stale", op: ">", value: 3 },
            ],
          },
        ],
      },
    });
  });

  it.each([
    [
      "open",
      [
        "above",
        "at or above",
        "below",
        "at or below",
        "is",
        "is not",
        "between",
      ],
    ],
    [
      "team",
      [
        "is",
        "is not",
        "contains",
        "starts with",
        "ends with",
        "is one of",
        "is none of",
      ],
    ],
    ["opened_at", ["before", "after", "between", "older than", "newer than"]],
  ])("offers the operators a %s column takes", async (column, operators) => {
    setUp(BLANK);

    await userEvent.selectOptions(leaf(1).getByLabelText("Column"), column);

    expect(optionsOf(leaf(1).getByLabelText("Operator")), column).toEqual(
      operators
    );
  });

  it.each([
    [
      { column: "open", op: "between", value: [1, 2] },
      "team",
      { column: "team", op: "==", value: "" },
    ],
    [
      { column: "open", op: "==", value: 4 },
      "team",
      { column: "team", op: "==", value: "" },
    ],
    [
      { column: "open", op: ">", value: 4 },
      "stale",
      { column: "stale", op: ">", value: 4 },
    ],
    [
      { column: "team", op: "contains", value: "x" },
      "opened_at",
      { column: "opened_at", op: "before", value: "" },
    ],
  ])(
    "picking another column keeps only what still fits: %j to %s",
    async (start, column, after) => {
      const latest = setUp({ rows: "one", where: start });

      await userEvent.selectOptions(leaf(1).getByLabelText("Column"), column);

      expect(latest()).toEqual({ rows: "one", where: after });
    }
  );

  it.each([
    ["between", "open", ["From", "To"]],
    ["in", "team", ["Values, one per line"]],
    ["older_than", "opened_at", ["Amount", "Unit"]],
    ["after", "opened_at", ["Value"]],
  ])("asks for the value %s takes", async (op, column, labels) => {
    setUp({
      rows: "one",
      where: { column, op: column === "team" ? "==" : "before", value: "" },
    });

    await userEvent.selectOptions(leaf(1).getByLabelText("Operator"), op);

    for (const label of labels) {
      expect(
        leaf(1).getByLabelText(label),
        `${op} should ask for ${label}`
      ).toBeInTheDocument();
    }
  });

  it.each([
    ["Amount", "7", "P7D"],
    ["Unit", "hours", "PT1H"],
  ])("writes a duration from its %s", async (label, entry, written) => {
    const latest = setUp({
      rows: "one",
      where: { column: "opened_at", op: "older_than", value: "P1D" },
    });

    const control = leaf(1).getByLabelText(label);
    if (control.tagName === "SELECT")
      await userEvent.selectOptions(control, entry);
    else {
      await userEvent.clear(control);
      await userEvent.type(control, entry);
    }

    expect(latest(), label).toMatchObject({ where: { value: written } });
  });

  it("writes an in list one value per line", async () => {
    const latest = setUp({
      rows: "one",
      where: { column: "team", op: "in", value: [] },
    });

    await userEvent.type(
      leaf(1).getByLabelText("Values, one per line"),
      "core{Enter}infra{Enter}"
    );

    expect(latest()).toMatchObject({ where: { value: ["core", "infra"] } });
    expect(leaf(1).getByLabelText("Values, one per line")).toHaveValue(
      "core\ninfra\n"
    );
  });

  it("keeps a wide integer as its digits", async () => {
    const latest = setUp({
      rows: "one",
      where: { column: "open", op: ">", value: "" },
    });

    await userEvent.type(
      leaf(1).getByLabelText("Value"),
      "12345678901234567890"
    );

    expect(latest()).toMatchObject({
      where: { value: "12345678901234567890" },
    });
  });

  it("hides the row quantifier while the metric answers one row", () => {
    setUp(BLANK, { manyRows: false });

    expect(screen.queryByLabelText("Rows")).toBeNull();
  });

  it("shows a quantifier the rule already holds even for one row", () => {
    setUp({ ...BLANK, rows: "all" }, { manyRows: false });

    expect(screen.getByLabelText("Rows")).toHaveValue("all");
  });

  it("counts matching rows when asked to", async () => {
    const latest = setUp(BLANK);

    await userEvent.selectOptions(screen.getByLabelText("Rows"), "count");
    await userEvent.selectOptions(
      screen.getByLabelText("Count comparison"),
      ">="
    );
    await userEvent.clear(screen.getByLabelText("Count"));
    await userEvent.type(screen.getByLabelText("Count"), "3");

    expect(latest()).toMatchObject({ rows: { count: { op: ">=", value: 3 } } });
  });

  it("offers no deeper group past the depth the service allows", () => {
    setUp({
      rows: "one",
      where: {
        all: [{ any: [{ all: [{ column: "open", op: ">", value: 1 }] }] }],
      },
    });

    const deepest = within(screen.getByRole("group", { name: "Group 2" }));
    expect(deepest.queryByRole("button", { name: "Add group" })).toBeNull();
    expect(
      deepest.getByRole("button", { name: "Add condition" })
    ).toBeEnabled();
  });

  it.each([
    [
      "all",
      "not",
      {
        all: [
          { column: "open", op: ">", value: 1 },
          { column: "stale", op: ">", value: 2 },
        ],
      },
      {
        not: {
          all: [
            { column: "open", op: ">", value: 1 },
            { column: "stale", op: ">", value: 2 },
          ],
        },
      },
    ],
    [
      "any",
      "not",
      { any: [{ column: "open", op: ">", value: 1 }] },
      { not: { column: "open", op: ">", value: 1 } },
    ],
    [
      "not",
      "any",
      { not: { column: "open", op: ">", value: 1 } },
      { any: [{ column: "open", op: ">", value: 1 }] },
    ],
  ])(
    "switching a group from %s to %s keeps its tests",
    async (_from, to, where, after) => {
      const latest = setUp({ rows: "one", where });

      await userEvent.selectOptions(screen.getByLabelText("Combine"), to);

      expect(latest()).toEqual({ rows: "one", where: after });
    }
  );

  it("removes a group with its last test", async () => {
    const latest = setUp({
      rows: "one",
      where: {
        all: [
          { column: "open", op: ">", value: 1 },
          { any: [{ column: "stale", op: ">", value: 2 }] },
        ],
      },
    });

    await userEvent.click(
      screen.getByRole("button", { name: "Remove condition 2" })
    );

    expect(latest()).toEqual({
      rows: "one",
      where: { all: [{ column: "open", op: ">", value: 1 }] },
    });
    expect(
      screen.queryByRole("button", { name: "Remove condition 1" })
    ).toBeNull();
  });

  it("describes both count controls with a refusal of the count", () => {
    setUp(
      {
        rows: { count: { op: ">", value: 3 } },
        where: { column: "open", op: ">", value: 1 },
      },
      { errors: { "condition.rows.count": "must hold op and value" } }
    );

    for (const label of ["Count comparison", "Count"]) {
      expect(screen.getByLabelText(label), label).toHaveAccessibleDescription(
        "must hold op and value"
      );
    }
    expect(screen.getByLabelText("Rows")).not.toHaveAttribute("aria-invalid");
  });

  it("puts each refusal on the control or group its path names", () => {
    setUp(
      {
        rows: { count: { op: ">", value: 3 } },
        where: {
          all: [{ column: "open", op: "contains", value: "1" }, { any: [] }],
        },
      },
      {
        errors: {
          "condition.where.all[0].op":
            "`contains` does not apply to a number column",
          "condition.where.all[1].any": "a group needs at least one condition",
          "condition.rows.count.value": "must be a whole number of rows",
          name: "Enter a name.",
        },
      }
    );

    expect(leaf(1).getByLabelText("Operator")).toHaveAccessibleDescription(
      "`contains` does not apply to a number column"
    );
    expect(leaf(1).getByLabelText("Column")).not.toHaveAttribute(
      "aria-invalid"
    );
    expect(
      within(screen.getByRole("group", { name: "Group 1" })).getByLabelText(
        "Combine"
      )
    ).toHaveAccessibleDescription("a group needs at least one condition");
    expect(screen.getByLabelText("Count")).toHaveAccessibleDescription(
      "must be a whole number of rows"
    );
    expect(screen.queryByText("Enter a name.")).toBeNull();
  });

  it.each([
    [
      "two tests at the top",
      {
        all: [
          { column: "open", op: ">", value: 1 },
          { column: "stale", op: ">", value: 2 },
        ],
      },
      false,
    ],
    [
      "two tests beside a group two deep",
      {
        all: [
          { column: "open", op: ">", value: 1 },
          {
            any: [
              { column: "stale", op: ">", value: 2 },
              { all: [{ column: "team", op: "==", value: "x" }] },
            ],
          },
        ],
      },
      true,
    ],
  ])("offers Not only within the depth bound: %s", (_case, where, refused) => {
    setUp({ rows: "one", where });

    const not = within(screen.getAllByLabelText("Combine")[0]).getByRole(
      "option",
      { name: "Not" }
    );
    expect((not as HTMLOptionElement).disabled, JSON.stringify(where)).toBe(
      refused
    );
  });

  it("asks for a column before the operator and value", () => {
    setUp(BLANK);

    expect(leaf(1).getByLabelText("Operator")).toBeDisabled();
    expect(leaf(1).getByLabelText("Value")).toBeDisabled();
  });

  it.each([
    ["the next test", 1, "Condition 1", "open"],
    ["the previous test when the last goes", 3, "Condition 2", "open"],
  ])(
    "moves focus to %s after a remove",
    async (_case, removed, focused, column) => {
      setUp({
        rows: "one",
        where: {
          all: [
            { column: "team", op: "==", value: "core" },
            { column: "open", op: ">", value: 1 },
            { column: "stale", op: ">", value: 2 },
          ],
        },
      });

      await userEvent.click(
        screen.getByRole("button", { name: `Remove condition ${removed}` })
      );

      const target = within(
        screen.getByRole("group", { name: focused })
      ).getByLabelText("Column");
      expect(target, `${focused} should hold focus`).toHaveFocus();
      expect(target).toHaveValue(column);
    }
  );

  it.each([
    ["Add condition", "Condition 2"],
    ["Add group", "Condition 2"],
  ])("moves focus to the new test's column after %s", async (button, added) => {
    setUp({ rows: "one", where: { column: "open", op: ">", value: 1 } });

    await userEvent.click(screen.getByRole("button", { name: button }));

    expect(
      leaf(Number(added.split(" ")[1])).getByLabelText("Column"),
      button
    ).toHaveFocus();
  });

  it("says why a part is not a condition, and resets the whole of it", async () => {
    const latest = setUp(
      { rows: "one", where: ["open"] },
      { errors: { "condition.where": "must be a group or a test" } }
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "must be a group or a test"
    );
    await userEvent.click(screen.getByRole("button", { name: "Reset" }));

    expect(latest()).toEqual({ rows: "one", where: blankLeaf() });
    expect(leaf(1).getByLabelText("Column")).toHaveFocus();
  });

  it("names every control", () => {
    setUp({
      rows: { count: { op: ">", value: 3 } },
      where: {
        all: [
          { column: "open", op: "between", value: [1, 2] },
          { not: { column: "opened_at", op: "older_than", value: "P7D" } },
          { column: "team", op: "in", value: ["core"] },
        ],
      },
    });

    for (const control of [
      ...screen.getAllByRole("combobox"),
      ...screen.getAllByRole("textbox"),
      ...screen.getAllByRole("button"),
    ]) {
      expect(control, control.outerHTML).toHaveAccessibleName();
    }
  });
});
