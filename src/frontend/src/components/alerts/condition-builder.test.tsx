import {
  fireEvent,
  render,
  screen,
  within,
  type BoundFunctions,
  type queries,
} from "@testing-library/react";
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
const ONLY_OPEN = new Map<string, ColumnKind>([["open", "number"]]);

function Harness({
  initial,
  kinds = KINDS,
  manyRows = true,
  errors = {},
  onChange,
}: {
  initial: unknown;
  kinds?: ReadonlyMap<string, ColumnKind>;
  manyRows?: boolean;
  errors?: FieldErrors;
  onChange: (condition: unknown) => void;
}) {
  const [condition, setCondition] = useState(initial);

  return (
    <ConditionBuilder
      condition={condition}
      columns={[...kinds.keys()]}
      kinds={kinds}
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

type Scope = BoundFunctions<typeof queries>;

/** Opens a kit select by its name and picks one of its options by label. */
async function pick(scope: Scope, control: string, option: string) {
  await userEvent.click(scope.getByRole("combobox", { name: control }));
  await userEvent.click(await screen.findByRole("option", { name: option }));
}

function isDisabled(option: HTMLElement): boolean {
  return (
    option.getAttribute("aria-disabled") === "true" ||
    option.hasAttribute("data-disabled")
  );
}

/** The options a kit select offers, read by opening it and closed again. */
async function optionsOf(trigger: HTMLElement): Promise<HTMLElement[]> {
  await userEvent.click(trigger);
  const options = await screen.findAllByRole("option");
  await userEvent.keyboard("{Escape}");
  return options;
}

function names(options: readonly HTMLElement[]): string[] {
  return options
    .filter((option) => !isDisabled(option))
    .map((option) => option.textContent ?? "");
}

const BLANK = { rows: "one", where: blankLeaf() };

describe("ConditionBuilder", () => {
  it("builds a nested condition with a quantifier through its controls", async () => {
    const latest = setUp(BLANK);

    await pick(screen, "Rows", "Any row");
    await pick(leaf(1), "Column", "open");
    await userEvent.type(leaf(1).getByLabelText("Value"), "10");
    await userEvent.click(screen.getByRole("button", { name: "Add group" }));
    const group = within(screen.getByRole("group", { name: "Group 1" }));
    await pick(group, "Combine", "Any of");
    await pick(leaf(2), "Column", "team");
    await userEvent.type(leaf(2).getByLabelText("Value"), "core");
    await userEvent.click(group.getByRole("button", { name: "Add condition" }));
    await pick(leaf(3), "Column", "stale");
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

    await pick(leaf(1), "Column", column);

    const offered = await optionsOf(leaf(1).getByLabelText("Operator"));
    expect(names(offered), column).toEqual(operators);
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

      await pick(leaf(1), "Column", column);

      expect(latest(), `${JSON.stringify(start)} to ${column}`).toEqual({
        rows: "one",
        where: after,
      });
    }
  );

  it.each([
    ["between", "open", ["From", "To"]],
    ["is one of", "team", ["Values"]],
    ["older than", "opened_at", ["Amount", "Unit"]],
    ["after", "opened_at", ["Value day", "Value time"]],
    ["between", "opened_at", ["From day", "From time", "To day", "To time"]],
  ])("asks for the value %s takes", async (op, column, labels) => {
    setUp({
      rows: "one",
      where: { column, op: column === "team" ? "==" : "before", value: "" },
    });

    await pick(leaf(1), "Operator", op);

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

    if (label === "Unit") await pick(leaf(1), label, entry);
    else {
      const control = leaf(1).getByLabelText(label);
      await userEvent.clear(control);
      await userEvent.type(control, entry);
    }

    expect(latest(), label).toMatchObject({ where: { value: written } });
  });

  describe("a list of values", () => {
    function listOf(value: unknown) {
      return setUp({ rows: "one", where: { column: "team", op: "in", value } });
    }

    function values() {
      return leaf(1).getByRole("combobox", { name: "Values" });
    }

    it.each([
      ["Enter", [], "core{Enter}infra{Enter}", ["core", "infra"]],
      ["a comma", [], "core,infra,", ["core", "infra"]],
      ["Enter, trimmed", [], "  core  {Enter}", ["core"]],
      ["Enter, empty", ["core"], "   {Enter}", undefined],
      ["Enter, a value already there", ["core"], "core{Enter}", undefined],
    ])("adds a value by %s", async (_case, start, typed, written) => {
      const latest = listOf(start);

      await userEvent.type(values(), typed);

      expect(
        (latest() as { where?: { value?: unknown } } | undefined)?.where?.value,
        _case
      ).toEqual(written);
      expect(screen.queryByRole("listbox"), "never opens a list").toBeNull();
    });

    it.each([
      [
        "past 50 values",
        Array.from({ length: 50 }, (_, at) => `team-${at}`),
        "extra",
        "At most 50 values.",
      ],
      ["past 256 characters", [], "x".repeat(257), "At most 256 characters."],
    ])(
      "refuses a value %s and keeps what was typed",
      async (_case, start, typed, said) => {
        const latest = listOf(start);

        await userEvent.click(values());
        await userEvent.paste(typed);
        await userEvent.keyboard("{Enter}");

        expect(latest(), _case).toBeUndefined();
        expect(values()).toHaveAccessibleDescription(said);
        expect(values()).toHaveValue(typed);
      }
    );

    it.each([
      [
        "its remove button",
        async () => {
          await userEvent.click(
            screen.getByRole("button", { name: "Remove core" })
          );
        },
        ["infra"],
      ],
      [
        "Backspace in an empty field",
        async () => {
          await userEvent.click(values());
          await userEvent.keyboard("{Backspace}");
        },
        ["core"],
      ],
    ])("removes a value by %s", async (_case, remove, kept) => {
      const latest = listOf(["core", "infra"]);

      await remove();

      expect(latest(), _case).toMatchObject({ where: { value: kept } });
    });

    it("shows a list written as JSON as it stands", () => {
      const latest = listOf(["", "core", "core", 7]);

      const removals = leaf(1)
        .getAllByRole("button")
        .map((button) => button.getAttribute("aria-label"));
      expect(removals).toEqual([
        "Remove ",
        "Remove core",
        "Remove core",
        "Remove 7",
      ]);
      expect(latest(), "nothing written").toBeUndefined();
    });
  });

  describe("a timestamp", () => {
    function at(value: unknown) {
      return setUp({
        rows: "one",
        where: { column: "opened_at", op: "before", value },
      });
    }

    it("shows a stored UTC day without shifting it to the local day", () => {
      at("2026-10-01T22:30:00Z");

      expect(leaf(1).getByLabelText("Value day")).toHaveTextContent(
        "October 1st, 2026"
      );
      expect(leaf(1).getByLabelText("Value time")).toHaveValue("22:30");
    });

    it("writes a picked day and a typed time as an RFC 3339 UTC timestamp", async () => {
      const latest = at("2026-10-01T22:30:00Z");

      await userEvent.click(leaf(1).getByLabelText("Value day"));
      await userEvent.click(
        await screen.findByRole("button", { name: /October 15th, 2026/ })
      );
      expect(latest()).toMatchObject({
        where: { value: "2026-10-15T22:30:00Z" },
      });

      fireEvent.change(leaf(1).getByLabelText("Value time"), {
        target: { value: "07:05" },
      });
      expect(latest()).toMatchObject({
        where: { value: "2026-10-15T07:05:00Z" },
      });
    });

    it("keeps a stored timestamp's seconds until its day or time changes", () => {
      const latest = at("2026-10-01T09:30:15.500Z");

      expect(leaf(1).getByLabelText("Value time")).toHaveValue("09:30");
      expect(latest(), "nothing written while untouched").toBeUndefined();

      fireEvent.change(leaf(1).getByLabelText("Value time"), {
        target: { value: "09:31" },
      });
      expect(latest()).toMatchObject({
        where: { value: "2026-10-01T09:31:00Z" },
      });
    });

    it("keeps an unfinished time in the field and writes nothing until it is whole", () => {
      const latest = at("2026-10-01T09:30:00Z");
      const time = leaf(1).getByLabelText("Value time");

      fireEvent.change(time, { target: { value: "" } });
      expect(time, "the cleared field is not refilled").toHaveValue("");
      expect(latest(), "no midnight is written").toBeUndefined();

      fireEvent.blur(time);
      expect(time, "leaving restores the stored time").toHaveValue("09:30");
      expect(latest()).toBeUndefined();
    });

    it("starts a picked day at midnight when no time is set, and shows it", async () => {
      const latest = at("");

      await userEvent.click(leaf(1).getByLabelText("Value day"));
      await userEvent.click(
        await screen.findByRole("button", { name: /October 15th, 2026/ })
      );

      expect(latest()).toMatchObject({
        where: { value: "2026-10-15T00:00:00Z" },
      });
      expect(leaf(1).getByLabelText("Value time")).toHaveValue("00:00");
    });

    it("writes nothing for a time until a day is picked", () => {
      const latest = at("");

      fireEvent.change(leaf(1).getByLabelText("Value time"), {
        target: { value: "09:30" },
      });

      expect(latest()).toBeUndefined();
      expect(leaf(1).getByLabelText("Value time")).toHaveValue("09:30");
    });
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

    expect(screen.getByLabelText("Rows")).toHaveTextContent("Every row");
  });

  it("counts matching rows when asked to", async () => {
    const latest = setUp(BLANK);

    await pick(screen, "Rows", "Number of rows");
    await pick(screen, "Count comparison", "at or above");
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
      "Not",
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
      "Not",
      { any: [{ column: "open", op: ">", value: 1 }] },
      { not: { column: "open", op: ">", value: 1 } },
    ],
    [
      "not",
      "Any of",
      { not: { column: "open", op: ">", value: 1 } },
      { any: [{ column: "open", op: ">", value: 1 }] },
    ],
  ])(
    "switching a group from %s to %s keeps its tests",
    async (_from, to, where, after) => {
      const latest = setUp({ rows: "one", where });

      await pick(screen, "Combine", to);

      expect(latest(), `${_from} to ${to}`).toEqual({
        rows: "one",
        where: after,
      });
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
  ])(
    "offers Not only within the depth bound: %s",
    async (_case, where, refused) => {
      setUp({ rows: "one", where });

      const offered = await optionsOf(screen.getAllByLabelText("Combine")[0]);
      const not = offered.find((option) => option.textContent === "Not");
      expect(not && isDisabled(not), JSON.stringify(where)).toBe(refused);
    }
  );

  it("shows the operator and value once a column is picked, and keeps focus on the column", async () => {
    setUp(BLANK);

    const column = leaf(1).getByRole("combobox", { name: "Column" });
    expect(column).toHaveTextContent("Pick a column");
    expect(column).toBeEnabled();
    for (const label of ["Operator", "Value"]) {
      expect(
        leaf(1).queryByLabelText(label),
        `${label} before a column`
      ).toBeNull();
    }

    await pick(leaf(1), "Column", "open");

    for (const label of ["Operator", "Value"]) {
      expect(
        leaf(1).getByLabelText(label),
        `${label} after a column`
      ).toBeEnabled();
    }
    expect(leaf(1).getByRole("combobox", { name: "Column" })).toHaveFocus();
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
      expect(target).toHaveTextContent(column);
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

  it.each([
    ["Add condition", "Condition 2"],
    ["Add group", "Condition 2"],
  ])(
    "starts a test added with %s on the metric's only column",
    async (button, added) => {
      const latest = setUp(
        { rows: "one", where: { column: "open", op: ">", value: 1 } },
        { kinds: ONLY_OPEN }
      );

      await userEvent.click(screen.getByRole("button", { name: button }));

      expect(
        leaf(2).getByRole("combobox", { name: "Column" }),
        added
      ).toHaveTextContent("open");
      expect(JSON.stringify(latest()), button).toContain(
        '{"column":"open","op":">","value":""}'
      );
    }
  );
});
