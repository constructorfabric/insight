import { useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import {
  COMPARISONS,
  MAX_GROUP_DEPTH,
  MAX_LEAVES,
  freshLeaf,
  canNegate,
  childPath,
  conditionParts,
  drawn,
  groupDepth,
  leafPaths,
  nodeAt,
  nodePaths,
  notPath,
  placeRefusals,
  rowsOf,
  type ColumnKind,
  type RowsChoice,
  type WrittenLeaf,
} from "@/lib/alerts/condition";
import type { FieldErrors } from "@/lib/alerts/draft";
import { saidId } from "@/lib/custom/editor/aria";
import { TEXT_BODY, TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { controlId } from "./condition-aria";
import { Choice, LeafEditor, NumberInput } from "./condition-leaf";

const ROWS: readonly { value: RowsChoice; label: string }[] = [
  { value: "one", label: "Exactly one row" },
  { value: "any", label: "Any row" },
  { value: "all", label: "Every row" },
  { value: "none", label: "No row" },
  { value: "count", label: "Number of rows" },
];

const JOINS = [
  { value: "all", label: "All of" },
  { value: "any", label: "Any of" },
  { value: "not", label: "Not" },
] as const;

type Join = (typeof JOINS)[number]["value"];

function rowsValue(choice: string): unknown {
  return choice === "count" ? { count: { op: ">", value: 1 } } : choice;
}

function childrenOfNot(child: unknown): readonly unknown[] {
  const read = drawn(child);
  return read.kind === "group" ? read.children : [child];
}

function rejoined(node: unknown, join: Join): unknown {
  const read = drawn(node);
  const children =
    read.kind === "group"
      ? read.children
      : read.kind === "not"
        ? childrenOfNot(read.child)
        : [node];

  if (join !== "not") return { [join]: children };
  if (read.kind === "not") return node;

  return children.length === 1
    ? { not: children[0] }
    : { not: { [read.kind === "group" ? read.join : "all"]: children } };
}

function firstControl(path: string): string[] {
  return [`${controlId(path)}-column`, controlId(path)];
}

function addConditionId(path: string): string {
  return `${controlId(path)}-add-condition`;
}

function addedFocus(path: string, added: unknown): string[] {
  const start = drawn(added).kind === "leaf" ? path : childPath(path, "all", 0);
  return firstControl(start);
}

interface Drawing {
  where: unknown;
  columns: readonly string[];
  kinds: ReadonlyMap<string, ColumnKind>;
  said: ReadonlyMap<string, Readonly<Record<string, string>>>;
  leafNumbers: ReadonlyMap<string, number>;
  groupNumbers: ReadonlyMap<string, number>;
  full: boolean;
  focus: (ids: readonly string[]) => void;
}

export function ConditionBuilder({
  condition,
  columns,
  kinds,
  manyRows,
  errors,
  onChange,
}: {
  condition: unknown;
  columns: readonly string[];
  kinds: ReadonlyMap<string, ColumnKind>;
  manyRows: boolean;
  errors: FieldErrors;
  onChange: (condition: unknown) => void;
}) {
  const pendingFocus = useRef<readonly string[]>([]);
  useEffect(() => {
    const wanted = pendingFocus.current;
    if (wanted.length === 0) return;
    pendingFocus.current = [];
    const found = wanted
      .map((id) => document.getElementById(id))
      .find((element) => element !== null);
    found?.focus();
  });

  const { rows, where } = conditionParts(condition);
  const placed = placeRefusals(errors, where);
  const leaves = leafPaths(where);
  const groups = nodePaths(where).filter((path) => {
    const kind = drawn(nodeAt(where, path)).kind;
    return path !== "where" && (kind === "group" || kind === "not");
  });
  const drawing: Drawing = {
    where,
    columns,
    kinds,
    said: placed.atNode,
    leafNumbers: new Map(leaves.map((path, index) => [path, index + 1])),
    groupNumbers: new Map(groups.map((path, index) => [path, index + 1])),
    full: leaves.length >= MAX_LEAVES,
    focus: (ids) => {
      pendingFocus.current = ids;
    },
  };
  const set = (patch: { rows?: unknown; where?: unknown }) =>
    onChange({
      ...(typeof condition === "object" && condition !== null ? condition : {}),
      ...patch,
    });

  return (
    <div className="flex flex-col gap-3">
      {placed.whole.map((message) => (
        <p
          key={message}
          role="alert"
          className={cn(TEXT_LABEL, "text-destructive")}
        >
          {message}
        </p>
      ))}
      <RowsEditor
        rows={rows}
        manyRows={manyRows}
        said={placed.rows}
        onChange={(next) => set({ rows: next })}
      />
      <NodeEditor
        path="where"
        node={where}
        drawing={drawing}
        root
        onChange={(next) => {
          if (next === undefined) drawing.focus(firstControl("where"));
          set({ where: next ?? freshLeaf(kinds) });
        }}
      />
    </div>
  );
}

function RowsEditor({
  rows,
  manyRows,
  said,
  onChange,
}: {
  rows: unknown;
  manyRows: boolean;
  said: Readonly<Record<string, string>>;
  onChange: (rows: unknown) => void;
}) {
  const read = rowsOf(rows);
  if (!manyRows && read.choice === "one" && Object.keys(said).length === 0) {
    return null;
  }

  const id = controlId("rows");
  const opId = `${id}-count-op`;
  const valueId = `${id}-count-value`;
  const messageIds: Record<string, string> = {
    rows: saidId(id),
    "rows.count": saidId(`${id}-count`),
    "rows.count.op": saidId(opId),
    "rows.count.value": saidId(valueId),
  };
  const messages = Object.entries(said).map(([path, message]) => ({
    id: messageIds[path] ?? saidId(id),
    message,
  }));
  const on = (...paths: string[]) =>
    messages
      .filter((each) => paths.some((path) => messageIds[path] === each.id))
      .map((each) => each.id);

  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-center gap-2">
        <label htmlFor={id} className={cn(TEXT_LABEL, "font-medium")}>
          Rows
        </label>
        <Choice
          id={id}
          label="Rows"
          value={read.choice ?? JSON.stringify(rows) ?? ""}
          options={ROWS}
          describedBy={on("rows")}
          className="w-44"
          onChange={(choice) => onChange(rowsValue(choice))}
        />
        {read.choice === "count" ? (
          <>
            <Choice
              id={opId}
              label="Count comparison"
              value={read.op ?? ""}
              options={COMPARISONS}
              describedBy={on("rows.count", "rows.count.op")}
              className="w-36"
              onChange={(op) => onChange({ count: { op, value: read.value } })}
            />
            <NumberInput
              id={valueId}
              label="Count"
              value={read.value}
              describedBy={on("rows.count", "rows.count.value")}
              onChange={(value) => onChange({ count: { op: read.op, value } })}
            />
          </>
        ) : null}
      </div>
      {messages.map(({ id: said, message }) => (
        <p
          key={`${said}${message}`}
          id={said}
          role="alert"
          className={cn(TEXT_LABEL, "text-destructive")}
        >
          {message}
        </p>
      ))}
    </div>
  );
}

function Messages({ id, said }: { id: string; said: readonly string[] }) {
  if (said.length === 0) return null;

  return (
    <p id={id} role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
      {said.join(" ")}
    </p>
  );
}

/** A node of the tree; `undefined` from `onChange` removes it. */
function NodeEditor({
  path,
  node,
  drawing,
  root = false,
  removable = false,
  onChange,
}: {
  path: string;
  node: unknown;
  drawing: Drawing;
  root?: boolean;
  removable?: boolean;
  onChange: (node: unknown) => void;
}) {
  const read = drawn(node);
  const said = drawing.said.get(path) ?? {};

  switch (read.kind) {
    case "leaf":
      return (
        <div className="flex flex-col gap-2">
          <LeafEditor
            path={path}
            leaf={read.leaf}
            number={drawing.leafNumbers.get(path) ?? 0}
            columns={drawing.columns}
            kinds={drawing.kinds}
            said={said}
            removable={removable}
            onChange={(leaf: WrittenLeaf) => onChange(leaf)}
            onRemove={() => onChange(undefined)}
          />
          {root ? (
            <Additions
              path={path}
              depth={1}
              full={drawing.full}
              kinds={drawing.kinds}
              onAdd={(added) => {
                drawing.focus(addedFocus(childPath(path, "all", 1), added));
                onChange({ all: [node, added] });
              }}
            />
          ) : null}
        </div>
      );
    case "group":
    case "not":
      return (
        <GroupEditor
          path={path}
          node={node}
          drawing={drawing}
          said={said}
          removable={removable}
          onChange={onChange}
        />
      );
    case "unreadable": {
      const id = controlId(path);
      return (
        <div className="flex flex-col gap-1">
          <div className="flex flex-wrap items-center gap-2">
            <p className={cn(TEXT_BODY, "text-destructive")}>
              This part is not a condition. Edit it in JSON.
            </p>
            {root ? (
              <Button
                id={id}
                type="button"
                variant="outline"
                size="sm"
                aria-describedby={
                  Object.keys(said).length > 0 ? saidId(id) : undefined
                }
                onClick={() => {
                  drawing.focus(firstControl(path));
                  onChange(freshLeaf(drawing.kinds));
                }}
              >
                Reset
              </Button>
            ) : removable ? (
              <Button
                id={id}
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => onChange(undefined)}
              >
                Remove
              </Button>
            ) : null}
          </div>
          <Messages id={saidId(id)} said={Object.values(said)} />
        </div>
      );
    }
  }
}

function Additions({
  path,
  depth,
  full,
  kinds,
  onAdd,
}: {
  path: string;
  depth: number;
  full: boolean;
  kinds: ReadonlyMap<string, ColumnKind>;
  onAdd: (node: unknown) => void;
}) {
  return (
    <div className="flex flex-wrap gap-2">
      <Button
        id={addConditionId(path)}
        type="button"
        variant="outline"
        size="sm"
        disabled={full}
        onClick={() => onAdd(freshLeaf(kinds))}
      >
        Add condition
      </Button>
      {depth < MAX_GROUP_DEPTH ? (
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={full}
          onClick={() => onAdd({ all: [freshLeaf(kinds)] })}
        >
          Add group
        </Button>
      ) : null}
    </div>
  );
}

function GroupEditor({
  path,
  node,
  drawing,
  said,
  removable,
  onChange,
}: {
  path: string;
  node: unknown;
  drawing: Drawing;
  said: Readonly<Record<string, string>>;
  removable: boolean;
  onChange: (node: unknown) => void;
}) {
  const read = drawn(node);
  const join: Join = read.kind === "group" ? read.join : "not";
  const depth = groupDepth(drawing.where, path);
  const negatable = canNegate(drawing.where, path);
  const id = controlId(path);
  const messages = Object.values(said);
  const number = drawing.groupNumbers.get(path);
  const name = number === undefined ? "Conditions" : `Group ${number}`;
  const children =
    read.kind === "group"
      ? read.children
      : read.kind === "not"
        ? [read.child]
        : [];
  const childAt = (index: number) =>
    read.kind === "group" ? childPath(path, read.join, index) : notPath(path);

  const replaceChild = (index: number, next: unknown) => {
    if (read.kind === "not") {
      onChange(next === undefined ? undefined : { not: next });
      return;
    }
    if (read.kind !== "group") return;

    const kept =
      next === undefined
        ? read.children.filter((_, at) => at !== index)
        : read.children.map((child, at) => (at === index ? next : child));
    if (next === undefined && kept.length > 0) {
      const sibling = Math.min(index, kept.length - 1);
      drawing.focus([
        ...firstControl(childPath(path, read.join, sibling)),
        addConditionId(path),
      ]);
    }
    onChange(kept.length === 0 ? undefined : { [read.join]: kept });
  };

  return (
    <div
      role="group"
      aria-label={name}
      className="flex flex-col gap-3 border-s-2 border-border ps-3"
    >
      <div className="flex flex-wrap items-center gap-2">
        <Choice
          id={id}
          label="Combine"
          value={join}
          options={JOINS.map((option) => ({
            ...option,
            disabled: option.value === "not" && !negatable,
          }))}
          describedBy={messages.length > 0 ? [saidId(id)] : []}
          className="w-28"
          onChange={(picked) => onChange(rejoined(node, picked as Join))}
        />
        {removable ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-label={`Remove ${name.toLowerCase()}`}
            onClick={() => onChange(undefined)}
          >
            Remove group
          </Button>
        ) : null}
      </div>
      <Messages id={saidId(id)} said={messages} />

      {children.map((child, index) => (
        <NodeEditor
          key={childAt(index)}
          path={childAt(index)}
          node={child}
          drawing={drawing}
          removable={path !== "where" || children.length > 1}
          onChange={(next) => replaceChild(index, next)}
        />
      ))}

      {read.kind === "group" ? (
        <Additions
          path={path}
          depth={depth}
          full={drawing.full}
          kinds={drawing.kinds}
          onAdd={(added) => {
            drawing.focus(
              addedFocus(
                childPath(path, read.join, read.children.length),
                added
              )
            );
            onChange({ [read.join]: [...read.children, added] });
          }}
        />
      ) : null}
    </div>
  );
}
