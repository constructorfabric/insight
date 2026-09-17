import type { ReactNode } from "react";

import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * The plain controls the editor is built from.
 *
 * A choice is a native `select` and a reference a native `input` with a
 * `datalist`: both are keyboard-reachable and carry their own label, which the
 * form needs on every one of the many rows a description can produce.
 */

const CONTROL = "h-9 w-full";

export function TextControl({
  id,
  value,
  placeholder,
  long,
  onChange,
}: {
  id: string;
  value: string;
  placeholder?: string;
  long?: boolean;
  onChange: (value: string) => void;
}) {
  const Control = long ? Textarea : Input;

  return (
    <Control
      id={id}
      value={value}
      placeholder={placeholder}
      className={long ? "min-h-20 w-full" : CONTROL}
      onChange={(event: { target: { value: string } }) =>
        onChange(event.target.value)
      }
    />
  );
}

export function NumberControl({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string;
  onChange: (value: number | undefined) => void;
}) {
  return (
    <Input
      id={id}
      type="number"
      value={value}
      className={CONTROL}
      onChange={(event) => {
        const written = event.target.value;
        onChange(written === "" ? undefined : Number(written));
      }}
    />
  );
}

export function FlagControl({
  id,
  value,
  onChange,
}: {
  id: string;
  value: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <input
      id={id}
      type="checkbox"
      checked={value}
      className="size-4 self-start accent-primary"
      onChange={(event) => onChange(event.target.checked)}
    />
  );
}

/** A fixed set, with the empty option standing for "not said". */
export function ChoiceControl({
  id,
  value,
  options,
  required,
  onChange,
}: {
  id: string;
  value: string;
  options: readonly string[];
  required?: boolean;
  onChange: (value: string | undefined) => void;
}) {
  return (
    <select
      id={id}
      value={value}
      className={cn(
        CONTROL,
        "rounded-md border border-input bg-transparent px-3 text-sm"
      )}
      onChange={(event) =>
        onChange(event.target.value === "" ? undefined : event.target.value)
      }
    >
      {required && value !== "" ? null : <option value="">—</option>}
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
    </select>
  );
}

/**
 * A stored definition's name.
 *
 * Offered from the catalogue but not restricted to it: the service decides
 * whether a name resolves, and a form that refused to let one be typed would
 * be wrong the moment the catalogue is a page behind.
 */
export function ReferenceControl({
  id,
  value,
  names,
  onChange,
}: {
  id: string;
  value: string;
  names: readonly string[];
  onChange: (value: string) => void;
}) {
  const listId = `${id}-names`;

  return (
    <>
      <Input
        id={id}
        value={value}
        list={listId}
        className={cn(CONTROL, "font-mono")}
        onChange={(event) => onChange(event.target.value)}
      />
      <datalist id={listId}>
        {names.map((name) => (
          <option key={name} value={name} />
        ))}
      </datalist>
    </>
  );
}

/** A label, its hint, the control, and whatever the service said about it. */
export function Row({
  id,
  label,
  hint,
  required,
  said,
  children,
}: {
  id: string;
  label: string;
  hint?: string;
  required?: boolean;
  said?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className={cn(TEXT_LABEL, "font-medium")}>
        <label htmlFor={id}>{label}</label>
        {required ? <span aria-hidden="true"> *</span> : null}
      </span>

      {children}

      {hint ? (
        <p className={cn(TEXT_LABEL, "text-muted-foreground")}>{hint}</p>
      ) : null}
      {said ? (
        <p role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
          {said}
        </p>
      ) : null}
    </div>
  );
}
