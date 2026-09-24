import type { ReactNode } from "react";

import { Input } from "@/components/ui/input";
import {
  hintId,
  noteId,
  saidId,
  type Describing,
} from "@/lib/custom/editor/aria";
import { Textarea } from "@/components/ui/textarea";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

const CONTROL = "h-9 w-full";

export function TextControl({
  id,
  value,
  placeholder,
  long,
  describe,
  onChange,
}: {
  id: string;
  value: string;
  placeholder?: string;
  long?: boolean;
  describe?: Describing;
  onChange: (value: string) => void;
}) {
  const Control = long ? Textarea : Input;

  return (
    <Control
      id={id}
      {...describe}
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
  describe,
  onChange,
}: {
  id: string;
  value: string;
  describe?: Describing;
  onChange: (value: number | undefined) => void;
}) {
  return (
    <Input
      id={id}
      {...describe}
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
  describe,
  onChange,
}: {
  id: string;
  value: boolean;
  describe?: Describing;
  onChange: (value: boolean) => void;
}) {
  return (
    <input
      id={id}
      {...describe}
      type="checkbox"
      checked={value}
      className="size-4 self-start accent-primary"
      onChange={(event) => onChange(event.target.checked)}
    />
  );
}

// Native rather than the kit's Select: a form draws dozens of these, and a
// test drives a native one with `selectOptions`.
export function ChoiceControl({
  id,
  value,
  options,
  required,
  describe,
  onChange,
}: {
  id: string;
  value: string;
  options: readonly string[];
  required?: boolean;
  describe?: Describing;
  onChange: (value: string | undefined) => void;
}) {
  return (
    <select
      id={id}
      {...describe}
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
      {options.includes(value) || value === "" ? null : (
        <option value={value}>{value}</option>
      )}
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
    </select>
  );
}

// INVARIANT: a name may be typed whether or not the catalogue lists it; the
// service decides whether it resolves, and the catalogue may be a page behind.
export function ReferenceControl({
  id,
  value,
  names,
  describe,
  onChange,
}: {
  id: string;
  value: string;
  names: readonly string[];
  describe?: Describing;
  onChange: (value: string) => void;
}) {
  const listId = `${id}-names`;

  return (
    <>
      <Input
        id={id}
        {...describe}
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

export function Row({
  id,
  label,
  property,
  hint,
  note,
  required,
  said,
  children,
}: {
  id: string;
  label: string;
  /** The property's name in the document, as the text view spells it. */
  property?: string;
  hint?: string;
  /** What the editor noticed, where the value is sendable but probably wrong. */
  note?: string;
  required?: boolean;
  said?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className={cn(TEXT_LABEL, "flex items-baseline gap-1 font-medium")}>
        <label htmlFor={id}>{label}</label>
        {property ? (
          <span
            aria-hidden="true"
            className="font-mono font-normal text-muted-foreground"
          >
            ({property})
          </span>
        ) : null}
        {required ? <span aria-hidden="true">*</span> : null}
      </span>

      {children}

      {hint ? (
        <p id={hintId(id)} className={cn(TEXT_LABEL, "text-muted-foreground")}>
          {hint}
        </p>
      ) : null}
      {note ? (
        <p id={noteId(id)} className={cn(TEXT_LABEL, "text-warning")}>
          {note}
        </p>
      ) : null}
      {said ? (
        <p
          id={saidId(id)}
          role="alert"
          className={cn(TEXT_LABEL, "text-destructive")}
        >
          {said}
        </p>
      ) : null}
    </div>
  );
}
