import { Plus, X } from "lucide-react";

import type { DeclaredField, EditableKind } from "@/api/custom-client";
import { Control } from "@/components/custom/editor/control";
import { Row } from "@/components/custom/editor/controls";
import { Variants } from "@/components/custom/editor/variants";
import { Button } from "@/components/ui/button";
import type { Field, Path, Shape } from "@/lib/custom/editor/describe";
import { spell } from "@/lib/custom/editor/describe";
import { describing } from "@/lib/custom/editor/aria";
import { blank } from "@/lib/custom/editor/blank";
import { read } from "@/lib/custom/editor/document";
import { alone } from "@/lib/custom/editor/exclusive";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

// INVARIANT: every row is drawn from the description, never from what the
// document happens to hold. A property the description does not know stays in
// the document untouched; the text view is where it is seen and edited.

export interface Editing {
  document: Record<string, unknown>;
  /** Whether this definition already exists, rather than being written now. */
  made: boolean;
  /** What the service said, by the path it named. */
  said: ReadonlyMap<string, string>;
  names: (kind: EditableKind) => readonly string[];
  /** What a dataset declares, for a metric's fields to be offered and typed by. */
  declared: (dataset: string) => readonly DeclaredField[];
  onChange: (path: Path, value: unknown) => void;
}

export function FieldsView({
  fields,
  at,
  editing,
  keepEmpty,
  ruled,
}: {
  fields: readonly Field[];
  at: Path;
  editing: Editing;
  /** Rows of one group, set apart from each other by a hairline. */
  ruled?: boolean;
  /**
   * INVARIANT: a record is told apart by the property it carries; removing it
   * when cleared would leave a record that is no variant, and nothing to show.
   */
  keepEmpty?: readonly string[];
}) {
  return (
    <div
      className={cn(
        "flex flex-col",
        ruled
          ? "divide-y divide-border [&>*]:py-3 [&>*:first-child]:pt-0 [&>*:last-child]:pb-0"
          : "gap-4"
      )}
    >
      {fields.map((field) => (
        <Settled key={field.name} field={field} editing={editing}>
          {field.shape.of === "variants" ? (
            <Variants
              field={field}
              shape={field.shape}
              at={at}
              editing={editing}
            />
          ) : (
            <FieldRow
              field={field}
              at={[...at, field.name]}
              editing={editing}
              keepEmpty={keepEmpty?.includes(field.name)}
            />
          )}
        </Settled>
      ))}
    </div>
  );
}

/**
 * A property the definition was made with, once it exists.
 *
 * A disabled fieldset takes every control under it out of reach, however deep
 * — which is what this needs, since what cannot be changed here is a whole
 * block rather than one input.
 */
function Settled({
  field,
  editing,
  children,
}: {
  field: Field;
  editing: Editing;
  children: React.ReactNode;
}) {
  if (field.atCreation !== true || !editing.made) return children;

  return (
    <fieldset disabled className="contents opacity-60">
      {children}
    </fieldset>
  );
}

function FieldRow({
  field,
  at,
  editing,
  keepEmpty,
}: {
  field: Field;
  at: Path;
  editing: Editing;
  keepEmpty?: boolean;
}) {
  if (field.shape.of === "record" || field.shape.of === "list") {
    return (
      <Nested field={field} at={at} editing={editing}>
        {field.shape.of === "record" ? (
          <FieldsView
            fields={field.shape.fields}
            at={at}
            editing={editing}
            ruled
          />
        ) : (
          <ListEntries shape={field.shape} at={at} editing={editing} />
        )}
      </Nested>
    );
  }

  const id = spell(at);
  const value = read(editing.document, at);
  const said = editing.said.get(id);
  const set = (next: unknown) => editing.onChange(at, next);
  const last = at.at(-1);

  return (
    <Row
      id={id}
      label={field.label}
      property={typeof last === "string" ? last : undefined}
      hint={field.hint}
      required={field.required}
      said={said}
    >
      <Control
        id={id}
        shape={field.shape}
        at={at}
        required={field.required}
        value={value}
        editing={editing}
        keepEmpty={keepEmpty}
        describe={describing(id, {
          hint: field.hint,
          said,
          required: field.required,
        })}
        onChange={
          field.shape.of === "flag" && field.shape.alone
            ? markAlone(at, editing)
            : set
        }
      />
    </Row>
  );
}

// A mark only one entry may carry moves; clearing it clears that entry alone.
function markAlone(at: Path, editing: Editing): (next: unknown) => void {
  const index = at.at(-2);
  const name = at.at(-1);
  const list = at.slice(0, -2);

  return (next) => {
    if (
      next !== true ||
      typeof index !== "number" ||
      typeof name !== "string"
    ) {
      editing.onChange(at, next);
      return;
    }
    editing.onChange(list, alone(read(editing.document, list), index, name));
  };
}

function Nested({
  field,
  at,
  editing,
  children,
}: {
  field: Field;
  at: Path;
  editing: Editing;
  children: React.ReactNode;
}) {
  const said = editing.said.get(spell(at));

  return (
    <fieldset className="flex flex-col gap-2 rounded-md border border-border p-3">
      <legend className={cn(TEXT_LABEL, "px-1 font-medium")}>
        {field.label}
        {field.required ? <span aria-hidden="true"> *</span> : null}
      </legend>

      {field.hint ? (
        <p className={cn(TEXT_LABEL, "text-muted-foreground")}>{field.hint}</p>
      ) : null}
      {said ? (
        <p role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
          {said}
        </p>
      ) : null}

      {children}
    </fieldset>
  );
}

function ListEntries({
  shape,
  at,
  editing,
}: {
  shape: Extract<Shape, { of: "list" }>;
  at: Path;
  editing: Editing;
}) {
  const held = read(editing.document, at);
  const entries = Array.isArray(held) ? held : [];

  return (
    <div className="flex flex-col gap-3">
      {entries.map((_, index) => (
        <div key={index} className="flex items-start gap-2">
          <div className="grow">
            <Entry
              called={`${shape.entryLabel} ${index + 1}`}
              shape={shape.entry}
              at={[...at, index]}
              editing={editing}
            />
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={`Remove ${shape.entryLabel} ${index + 1}`}
            onClick={() => editing.onChange([...at, index], undefined)}
          >
            <X />
          </Button>
        </div>
      ))}

      <Button
        variant="outline"
        size="sm"
        className="self-start"
        onClick={() =>
          editing.onChange(
            [...at, entries.length],
            blank(shape.entry, editing.document)
          )
        }
      >
        <Plus />
        Add {shape.entryLabel}
      </Button>
    </div>
  );
}

function Entry({
  called,
  shape,
  at,
  editing,
}: {
  called: string;
  shape: Shape;
  at: Path;
  editing: Editing;
}) {
  if (shape.of === "record" || shape.of === "variants") {
    const said = editing.said.get(spell(at));

    return (
      <fieldset className="flex flex-col gap-3 rounded-md border border-border p-3">
        <legend className={cn(TEXT_LABEL, "px-1 font-medium")}>{called}</legend>

        {said ? (
          <p role="alert" className={cn(TEXT_LABEL, "text-destructive")}>
            {said}
          </p>
        ) : null}

        {shape.of === "record" ? (
          <FieldsView fields={shape.fields} at={at} editing={editing} ruled />
        ) : (
          <Variants
            field={{ name: called, label: "Kind", shape }}
            shape={shape}
            at={at}
            editing={editing}
          />
        )}
      </fieldset>
    );
  }

  // An entry emptied is an entry still: a list shortens when its entry is
  // removed, not when what it holds is cleared.
  return (
    <FieldRow
      field={{ name: called, label: called, shape }}
      at={at}
      editing={editing}
      keepEmpty
    />
  );
}
