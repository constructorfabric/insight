import type {
  ComparisonOperator,
  ConditionNode,
  ConditionOperator,
} from "@/api/alerts-types";

export type ColumnKind = "number" | "text" | "date";

interface OperatorOption<T extends string = ConditionOperator> {
  value: T;
  label: string;
}

// INVARIANT: the service bounds nesting counting `all`, `any` and `not` alike.
export const MAX_GROUP_DEPTH = 3;
export const MAX_LEAVES = 20;

export const COMPARISONS: readonly OperatorOption<ComparisonOperator>[] = [
  { value: ">", label: "above" },
  { value: ">=", label: "at or above" },
  { value: "<", label: "below" },
  { value: "<=", label: "at or below" },
  { value: "==", label: "is" },
  { value: "!=", label: "is not" },
];

export const OPERATORS: Readonly<
  Record<ColumnKind, readonly OperatorOption[]>
> = {
  number: [...COMPARISONS, { value: "between", label: "between" }],
  text: [
    { value: "==", label: "is" },
    { value: "!=", label: "is not" },
    { value: "contains", label: "contains" },
    { value: "starts_with", label: "starts with" },
    { value: "ends_with", label: "ends with" },
    { value: "in", label: "is one of" },
    { value: "not_in", label: "is none of" },
  ],
  date: [
    { value: "before", label: "before" },
    { value: "after", label: "after" },
    { value: "between", label: "between" },
    { value: "older_than", label: "older than" },
    { value: "newer_than", label: "newer than" },
  ],
};

export type ValueShape =
  | "number"
  | "number-pair"
  | "text"
  | "text-list"
  | "moment"
  | "moment-pair"
  | "age";

export function fits(kind: ColumnKind, op: string): boolean {
  return OPERATORS[kind].some((option) => option.value === op);
}

export function valueShape(kind: ColumnKind, op: string): ValueShape {
  switch (kind) {
    case "number":
      return op === "between" ? "number-pair" : "number";
    case "text":
      return op === "in" || op === "not_in" ? "text-list" : "text";
    case "date":
      if (op === "between") return "moment-pair";
      return op === "older_than" || op === "newer_than" ? "age" : "moment";
  }
}

export function blankValue(shape: ValueShape): unknown {
  switch (shape) {
    case "number":
    case "text":
    case "moment":
      return "";
    case "number-pair":
    case "moment-pair":
      return ["", ""];
    case "text-list":
      return [];
    case "age":
      return "P1D";
  }
}

export function blankLeaf(): ConditionNode {
  return { column: "", op: ">", value: "" };
}

/** A new test: on the metric's only column when it has one, else blank. */
export function freshLeaf(
  kinds: ReadonlyMap<string, ColumnKind>
): ConditionNode {
  const [only, ...others] = [...kinds];
  if (only === undefined || others.length > 0) return blankLeaf();

  const [column, kind] = only;
  const op = OPERATORS[kind][0].value;
  return { column, op, value: blankValue(valueShape(kind, op)) };
}

export interface WrittenLeaf {
  column: string;
  op: string;
  value: unknown;
}

type Drawn =
  | { kind: "leaf"; leaf: WrittenLeaf }
  | { kind: "group"; join: "all" | "any"; children: readonly unknown[] }
  | { kind: "not"; child: unknown }
  | { kind: "unreadable" };

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

export function drawn(value: unknown): Drawn {
  const node = record(value);
  if (node === undefined) return { kind: "unreadable" };

  if (Array.isArray(node.all))
    return { kind: "group", join: "all", children: node.all };
  if (Array.isArray(node.any))
    return { kind: "group", join: "any", children: node.any };
  if ("not" in node) return { kind: "not", child: node.not };
  if (typeof node.column !== "string" || typeof node.op !== "string") {
    return { kind: "unreadable" };
  }

  return {
    kind: "leaf",
    leaf: { column: node.column, op: node.op, value: node.value },
  };
}

export type RowsChoice = "one" | "any" | "all" | "none" | "count";

interface ReadRows {
  choice: RowsChoice | undefined;
  op?: string;
  value?: unknown;
}

export function rowsOf(rows: unknown): ReadRows {
  if (rows === "one" || rows === "any" || rows === "all" || rows === "none") {
    return { choice: rows };
  }

  const count = record(record(rows)?.count);
  if (count === undefined) return { choice: undefined };

  return { choice: "count", op: String(count.op ?? ""), value: count.value };
}

export function conditionParts(condition: unknown): {
  rows: unknown;
  where: unknown;
} {
  const read = record(condition);
  return { rows: read?.rows, where: read?.where };
}

export function childPath(
  path: string,
  join: "all" | "any",
  index: number
): string {
  return `${path}.${join}[${index}]`;
}

export function notPath(path: string): string {
  return `${path}.not`;
}

export function leafPaths(where: unknown, path = "where"): string[] {
  const node = drawn(where);

  switch (node.kind) {
    case "leaf":
      return [path];
    case "group":
      return node.children.flatMap((child, index) =>
        leafPaths(child, childPath(path, node.join, index))
      );
    case "not":
      return leafPaths(node.child, notPath(path));
    case "unreadable":
      return [];
  }
}

export function nodePaths(where: unknown, path = "where"): string[] {
  const node = drawn(where);

  switch (node.kind) {
    case "leaf":
    case "unreadable":
      return [path];
    case "group":
      return [
        path,
        ...node.children.flatMap((child, index) =>
          nodePaths(child, childPath(path, node.join, index))
        ),
      ];
    case "not":
      return [path, ...nodePaths(node.child, notPath(path))];
  }
}

export function leavesOf(
  condition: unknown
): { path: string; leaf: WrittenLeaf }[] {
  const { where } = conditionParts(condition);

  return leafPaths(where).flatMap((path) => {
    const node = drawn(nodeAt(where, path));
    return node.kind === "leaf" ? [{ path, leaf: node.leaf }] : [];
  });
}

export function conditionColumns(condition: unknown): string[] {
  const named = leavesOf(condition)
    .map(({ leaf }) => leaf.column)
    .filter((column) => column !== "");

  return [...new Set(named)];
}

const STEP = /\.(all|any)\[(\d+)\]|\.not/g;

export function nodeAt(where: unknown, path: string): unknown {
  let at = where;
  for (const step of path.slice("where".length).matchAll(STEP)) {
    const node = record(at);
    if (node === undefined) return undefined;
    at = step[1]
      ? (node[step[1]] as unknown[] | undefined)?.[Number(step[2])]
      : node.not;
  }

  return at;
}

export function groupDepth(where: unknown, path: string): number {
  const node = drawn(nodeAt(where, path));
  const own = node.kind === "group" || node.kind === "not" ? 1 : 0;
  const above = [...path.matchAll(STEP)].length;

  return above + own;
}

export function pathId(path: string): string {
  return path.replace(/[^A-Za-z0-9]+/g, "-").replace(/-+$/, "");
}

function groupHeight(node: unknown): number {
  const read = drawn(node);

  switch (read.kind) {
    case "leaf":
    case "unreadable":
      return 0;
    case "group":
      return 1 + Math.max(0, ...read.children.map(groupHeight));
    case "not":
      return 1 + groupHeight(read.child);
  }
}

export function canNegate(where: unknown, path: string): boolean {
  const node = nodeAt(where, path);
  const read = drawn(node);
  if (read.kind === "not") return true;
  if (read.kind !== "group" || read.children.length <= 1) return true;

  const above = groupDepth(where, path) - 1;
  return above + 1 + groupHeight(node) <= MAX_GROUP_DEPTH;
}

function fitNode(
  node: unknown,
  kinds: ReadonlyMap<string, ColumnKind>
): unknown {
  const read = drawn(node);

  switch (read.kind) {
    case "leaf": {
      const kind = kinds.get(read.leaf.column);
      return kind !== undefined && fits(kind, read.leaf.op)
        ? node
        : freshLeaf(kinds);
    }
    case "group":
      return {
        [read.join]: read.children.map((child) => fitNode(child, kinds)),
      };
    case "not":
      return { not: fitNode(read.child, kinds) };
    case "unreadable":
      return node;
  }
}

export function fitCondition(
  condition: unknown,
  kinds: ReadonlyMap<string, ColumnKind>
): unknown {
  const read = record(condition);
  if (read === undefined) return condition;

  return { ...read, where: fitNode(read.where, kinds) };
}

const PREFIX = "condition.";

export interface PlacedRefusals {
  atNode: ReadonlyMap<string, Readonly<Record<string, string>>>;
  rows: Readonly<Record<string, string>>;
  whole: readonly string[];
}

export function placeRefusals(
  errors: Readonly<Record<string, string>>,
  where: unknown
): PlacedRefusals {
  const nodes = nodePaths(where).sort((a, b) => b.length - a.length);
  const atNode = new Map<string, Record<string, string>>();
  const rows: Record<string, string> = {};
  const whole: string[] = [];

  for (const [field, message] of Object.entries(errors)) {
    if (field !== "condition" && !field.startsWith(PREFIX)) continue;
    const inner = field.slice(PREFIX.length);

    if (inner === "rows" || inner.startsWith("rows.")) {
      rows[inner] = message;
      continue;
    }
    const owner = nodes.find(
      (path) =>
        inner === path ||
        inner.startsWith(`${path}.`) ||
        inner.startsWith(`${path}[`)
    );
    if (owner === undefined) {
      whole.push(message);
      continue;
    }
    atNode.set(owner, { ...atNode.get(owner), [inner]: message });
  }

  return { atNode, rows, whole };
}

export const MAX_LIST_VALUES = 50;
export const MAX_TEXT_CHARS = 256;

/**
 * A list with entries added: trimmed, and neither empty nor already there.
 * An entry past a bound is refused with why; it and the entries after it
 * are left unadded.
 */
export function withListValues(
  current: readonly unknown[],
  entries: readonly string[]
): { values: unknown[]; refused?: string; unadded: string[] } {
  const values = [...current];

  for (const [index, entry] of entries.entries()) {
    const value = entry.trim();
    if (value === "" || values.includes(value)) continue;

    const unadded = entries.slice(index);
    // INVARIANT: the service counts characters, and `length` counts UTF-16 units.
    if ([...value].length > MAX_TEXT_CHARS) {
      return {
        values,
        refused: `At most ${MAX_TEXT_CHARS} characters.`,
        unadded,
      };
    }
    if (values.length >= MAX_LIST_VALUES) {
      return { values, refused: `At most ${MAX_LIST_VALUES} values.`, unadded };
    }
    values.push(value);
  }

  return { values, unadded: [] };
}
