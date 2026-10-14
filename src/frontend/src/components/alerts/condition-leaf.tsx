import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  OPERATORS,
  blankValue,
  fits,
  valueShape,
  type ColumnKind,
  type ValueShape,
  type WrittenLeaf,
} from "@/lib/alerts/condition";
import { numberOf } from "@/lib/alerts/draft";
import { saidId } from "@/lib/custom/editor/aria";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

import { controlId, described } from "./condition-aria";
import { FieldSelect } from "./field-select";

interface Option {
  value: string;
  label: string;
  disabled?: boolean;
}

// INVARIANT: a value the list does not offer is shown as an option of its
// own, so a draft written as JSON is drawn as it stands, never rewritten.
export function Choice({
  id,
  label,
  value,
  options,
  placeholder,
  describedBy = [],
  disabled,
  className,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  options: readonly Option[];
  placeholder?: string;
  describedBy?: readonly string[];
  disabled?: boolean;
  className?: string;
  onChange: (value: string) => void;
}) {
  const offered =
    value === "" || options.some((option) => option.value === value);

  return (
    <FieldSelect
      id={id}
      label={label}
      value={value}
      options={offered ? options : [{ value, label: value }, ...options]}
      placeholder={placeholder}
      disabled={disabled}
      describe={described(describedBy)}
      className={className}
      onChange={onChange}
    />
  );
}

function typedNumber(typed: string): unknown {
  return numberOf(typed) ?? typed;
}

function shownNumber(value: unknown): string {
  return typeof value === "number" || typeof value === "string"
    ? String(value)
    : "";
}

// INVARIANT: what is typed is kept while it still means the value held, so
// "1.50" is not rewritten to "1.5" mid-word.
export function NumberInput({
  id,
  label,
  value,
  describedBy = [],
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  value: unknown;
  describedBy?: readonly string[];
  disabled?: boolean;
  onChange: (value: unknown) => void;
}) {
  const [typed, setTyped] = useState(() => shownNumber(value));
  const shown = typedNumber(typed) === value ? typed : shownNumber(value);

  return (
    <Input
      id={id}
      aria-label={label}
      {...described(describedBy)}
      inputMode="decimal"
      value={shown}
      disabled={disabled}
      className="h-9 w-32 tabular-nums"
      onChange={(event) => {
        setTyped(event.target.value);
        onChange(typedNumber(event.target.value));
      }}
    />
  );
}

function linesOf(typed: string): string[] {
  return typed.split("\n").filter((line) => line !== "");
}

function sameList(left: readonly string[], right: unknown): boolean {
  return (
    Array.isArray(right) &&
    left.length === right.length &&
    left.every((item, index) => item === right[index])
  );
}

function TextListInput({
  id,
  value,
  describedBy,
  disabled,
  onChange,
}: {
  id: string;
  value: unknown;
  describedBy: readonly string[];
  disabled: boolean;
  onChange: (value: string[]) => void;
}) {
  const listed = Array.isArray(value) ? value.map(String).join("\n") : "";
  const [typed, setTyped] = useState(listed);
  const shown = sameList(linesOf(typed), value) ? typed : listed;

  return (
    <Textarea
      id={id}
      aria-label="Values, one per line"
      {...described(describedBy)}
      value={shown}
      placeholder="One per line"
      disabled={disabled}
      rows={2}
      className="min-h-9 w-48"
      onChange={(event) => {
        setTyped(event.target.value);
        onChange(linesOf(event.target.value));
      }}
    />
  );
}

const TEXT = "h-9 w-48";
const MOMENT_PLACEHOLDER = "2026-10-01T09:00:00Z";

const AGE_UNITS: readonly (Option & { prefix: string; suffix: string })[] = [
  { value: "minutes", label: "minutes", prefix: "PT", suffix: "M" },
  { value: "hours", label: "hours", prefix: "PT", suffix: "H" },
  { value: "days", label: "days", prefix: "P", suffix: "D" },
  { value: "weeks", label: "weeks", prefix: "P", suffix: "W" },
];

function ageOf(value: unknown): { amount: string; unit: string } {
  const read =
    typeof value === "string" ? /^P(T?)(.*?)([MHDW]?)$/.exec(value) : null;
  if (!read) return { amount: "", unit: "days" };

  const timed = read[1] === "T";
  const unit =
    AGE_UNITS.find(
      (each) => each.suffix === read[3] && (each.prefix === "PT") === timed
    )?.value ?? "days";

  return { amount: read[2], unit };
}

function ageText(amount: string, unit: string): string {
  const picked = AGE_UNITS.find((each) => each.value === unit) ?? AGE_UNITS[2];
  return `${picked.prefix}${amount.trim()}${picked.suffix}`;
}

function at(value: unknown, index: number): unknown {
  return Array.isArray(value) ? value[index] : undefined;
}

function withAt(value: unknown, index: number, item: unknown): unknown[] {
  const pair = Array.isArray(value) ? [...value] : ["", ""];
  pair[index] = item;
  return pair;
}

function textOfValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function ValueInput({
  id,
  shape,
  value,
  saidOn,
  disabled,
  onChange,
}: {
  id: string;
  shape: ValueShape;
  value: unknown;
  saidOn: (end?: 0 | 1) => string[];
  disabled: boolean;
  onChange: (value: unknown) => void;
}) {
  switch (shape) {
    case "number":
      return (
        <NumberInput
          id={id}
          label="Value"
          value={value}
          describedBy={saidOn()}
          disabled={disabled}
          onChange={onChange}
        />
      );
    case "number-pair":
      return (
        <span className="flex items-center gap-2">
          <NumberInput
            id={`${id}-0`}
            label="From"
            value={at(value, 0)}
            describedBy={saidOn(0)}
            disabled={disabled}
            onChange={(low) => onChange(withAt(value, 0, low))}
          />
          <span className={TEXT_LABEL}>and</span>
          <NumberInput
            id={`${id}-1`}
            label="To"
            value={at(value, 1)}
            describedBy={saidOn(1)}
            disabled={disabled}
            onChange={(high) => onChange(withAt(value, 1, high))}
          />
        </span>
      );
    case "text":
      return (
        <Input
          id={id}
          aria-label="Value"
          {...described(saidOn())}
          disabled={disabled}
          value={textOfValue(value)}
          className={TEXT}
          onChange={(event) => onChange(event.target.value)}
        />
      );
    case "text-list":
      return (
        <TextListInput
          id={id}
          value={value}
          describedBy={saidOn()}
          disabled={disabled}
          onChange={onChange}
        />
      );
    case "moment":
      return (
        <Input
          id={id}
          aria-label="Value"
          {...described(saidOn())}
          disabled={disabled}
          value={textOfValue(value)}
          placeholder={MOMENT_PLACEHOLDER}
          className={cn(TEXT, "font-mono")}
          onChange={(event) => onChange(event.target.value)}
        />
      );
    case "moment-pair":
      return (
        <span className="flex flex-wrap items-center gap-2">
          {([0, 1] as const).map((end) => (
            <Input
              key={end}
              id={`${id}-${end}`}
              aria-label={end === 0 ? "From" : "To"}
              {...described(saidOn(end))}
              disabled={disabled}
              value={textOfValue(at(value, end))}
              placeholder={MOMENT_PLACEHOLDER}
              className={cn(TEXT, "font-mono")}
              onChange={(event) =>
                onChange(withAt(value, end, event.target.value))
              }
            />
          ))}
        </span>
      );
    case "age": {
      const { amount, unit } = ageOf(value);
      return (
        <span className="flex items-center gap-2">
          <Input
            id={id}
            aria-label="Amount"
            {...described(saidOn())}
            disabled={disabled}
            inputMode="numeric"
            value={amount}
            className="h-9 w-20 tabular-nums"
            onChange={(event) => onChange(ageText(event.target.value, unit))}
          />
          <Choice
            id={`${id}-unit`}
            label="Unit"
            value={unit}
            options={AGE_UNITS}
            disabled={disabled}
            onChange={(picked) => onChange(ageText(amount, picked))}
          />
        </span>
      );
    }
  }
}

const TEXT_OPERATORS: ReadonlySet<string> = new Set([
  "contains",
  "starts_with",
  "ends_with",
  "in",
  "not_in",
]);

function kindOf(
  leaf: WrittenLeaf,
  kinds: ReadonlyMap<string, ColumnKind>
): ColumnKind {
  const known = kinds.get(leaf.column);
  if (known) return known;
  if (TEXT_OPERATORS.has(leaf.op)) return "text";
  if (fits("date", leaf.op) && leaf.op !== "between") return "date";

  const sample = Array.isArray(leaf.value) ? leaf.value[0] : leaf.value;
  if (typeof sample !== "string" || numberOf(sample) !== undefined) {
    return "number";
  }

  return leaf.op === "between" ? "date" : "text";
}

function withColumn(
  leaf: WrittenLeaf,
  column: string,
  kinds: ReadonlyMap<string, ColumnKind>
): WrittenLeaf {
  const was = kindOf(leaf, kinds);
  const next = kinds.get(column);
  if (next === undefined) return { ...leaf, column };
  if (!fits(next, leaf.op)) {
    const op = OPERATORS[next][0].value;
    return { column, op, value: blankValue(valueShape(next, op)) };
  }

  const value =
    next === was ? leaf.value : blankValue(valueShape(next, leaf.op));
  return { column, op: leaf.op, value };
}

function withOperator(
  leaf: WrittenLeaf,
  op: string,
  kind: ColumnKind
): WrittenLeaf {
  const same = valueShape(kind, leaf.op) === valueShape(kind, op);
  return {
    ...leaf,
    op,
    value: same ? leaf.value : blankValue(valueShape(kind, op)),
  };
}

type LeafPart = "column" | "op" | "value" | "value-0" | "value-1" | "leaf";

function partOf(suffix: string, paired: boolean): LeafPart {
  if (suffix === ".column") return "column";
  if (suffix === ".op") return "op";
  if (paired && suffix === ".value[0]") return "value-0";
  if (paired && suffix === ".value[1]") return "value-1";
  if (suffix === ".value" || suffix.startsWith(".value[")) return "value";
  return "leaf";
}

const PART_ORDER: readonly LeafPart[] = [
  "leaf",
  "column",
  "op",
  "value",
  "value-0",
  "value-1",
];

export function LeafEditor({
  path,
  leaf,
  number,
  columns,
  kinds,
  said,
  removable,
  onChange,
  onRemove,
}: {
  path: string;
  leaf: WrittenLeaf;
  number: number;
  columns: readonly string[];
  kinds: ReadonlyMap<string, ColumnKind>;
  said: Readonly<Record<string, string>>;
  removable: boolean;
  onChange: (leaf: WrittenLeaf) => void;
  onRemove: () => void;
}) {
  const kind = kindOf(leaf, kinds);
  const shape = valueShape(kind, leaf.op);
  const id = controlId(path);
  const ids: Record<LeafPart, string> = {
    leaf: id,
    column: `${id}-column`,
    op: `${id}-op`,
    value: `${id}-value`,
    "value-0": `${id}-value-0`,
    "value-1": `${id}-value-1`,
  };

  const messages = new Map<LeafPart, string>();
  for (const [key, message] of Object.entries(said)) {
    const paired = shape === "number-pair" || shape === "moment-pair";
    const part = partOf(key.slice(path.length), paired);
    messages.set(part, [messages.get(part), message].filter(Boolean).join(" "));
  }
  const saidOf = (part: LeafPart) =>
    messages.has(part) ? [saidId(ids[part])] : [];
  const saidOnValue = (end?: 0 | 1) => [
    ...saidOf("value"),
    ...(end === undefined ? [] : saidOf(end === 0 ? "value-0" : "value-1")),
  ];

  const unpicked = leaf.column === "";
  const noColumns = columns.length === 0;
  const columnOptions = columns.map((name) => ({ value: name, label: name }));

  return (
    <div
      role="group"
      aria-label={`Condition ${number}`}
      className="flex flex-col gap-1"
    >
      <div className="flex flex-wrap items-center gap-2">
        <Choice
          id={ids.column}
          label="Column"
          value={leaf.column}
          options={columnOptions}
          placeholder={noColumns ? "Pick a metric first" : "Pick a column"}
          disabled={noColumns}
          describedBy={[...saidOf("leaf"), ...saidOf("column")]}
          className="w-44 font-mono"
          onChange={(column) => onChange(withColumn(leaf, column, kinds))}
        />
        <Choice
          id={ids.op}
          label="Operator"
          value={unpicked ? "" : leaf.op}
          options={OPERATORS[kind]}
          describedBy={saidOf("op")}
          disabled={unpicked}
          className="w-36"
          onChange={(op) => onChange(withOperator(leaf, op, kind))}
        />
        <ValueInput
          key={shape}
          id={ids.value}
          shape={shape}
          value={leaf.value}
          saidOn={saidOnValue}
          disabled={unpicked}
          onChange={(value) => onChange({ ...leaf, value })}
        />
        {removable ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-label={`Remove condition ${number}`}
            onClick={onRemove}
          >
            Remove
          </Button>
        ) : null}
      </div>
      {PART_ORDER.filter((part) => messages.has(part)).map((part) => (
        <p
          key={part}
          id={saidId(ids[part])}
          role="alert"
          className={cn(TEXT_LABEL, "text-destructive")}
        >
          {messages.get(part)}
        </p>
      ))}
    </div>
  );
}
