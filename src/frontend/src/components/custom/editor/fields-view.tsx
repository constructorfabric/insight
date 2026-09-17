import { Plus, X } from "lucide-react";

import type { EditableKind } from "@/api/custom-client";
import {
  ChoiceControl,
  FlagControl,
  NumberControl,
  ReferenceControl,
  Row,
  TextControl,
} from "@/components/custom/editor/controls";
import { Variants } from "@/components/custom/editor/variants";
import { Button } from "@/components/ui/button";
import type { Field, Path, Shape } from "@/lib/custom/editor/describe";
import { spell } from "@/lib/custom/editor/describe";
import { read } from "@/lib/custom/editor/document";
import { TEXT_LABEL } from "@/lib/type-scale";
import { cn } from "@/lib/utils";

/**
 * A document shown as the fields its kind admits.
 *
 * INVARIANT: every row is drawn from the description, never from what the
 * document happens to hold. A property the description does not know stays in
 * the document untouched — the text view is where it is seen and edited.
 */

/** Everything a row needs: what is held, what was said, and where to send it. */
export interface Editing {
  document: Record<string, unknown>;
  /** What the service said about each path it named. */
  said: ReadonlyMap<string, string>;
  /** The stored names of a kind, for a reference to offer. */
  names: (kind: EditableKind) => readonly string[];
  onChange: (path: Path, value: unknown) => void;
}

export function FieldsView({
  fields,
  at,
  editing,
  keepEmpty,
}: {
  fields: readonly Field[];
  at: Path;
  editing: Editing;
  /**
   * Fields whose emptiness is a value rather than an absence.
   *
   * INVARIANT: a record is told apart by the property it carries, so removing
   * that property when it is cleared would leave a record that is no variant
   * at all and a form with nothing left to show.
   */
  keepEmpty?: readonly string[];
}) {
  return (
    <div className="flex flex-col gap-4">
      {fields.map((field) =>
        field.shape.of === "variants" ? (
          <Variants
            key={field.name}
            field={field}
            shape={field.shape}
            at={at}
            editing={editing}
          />
        ) : (
          <FieldRow
            key={field.name}
            field={field}
            at={[...at, field.name]}
            editing={editing}
            keepEmpty={keepEmpty?.includes(field.name)}
          />
        )
      )}
    </div>
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
          <FieldsView fields={field.shape.fields} at={at} editing={editing} />
        ) : (
          <ListEntries shape={field.shape} at={at} editing={editing} />
        )}
      </Nested>
    );
  }

  const id = spell(at);
  const value = read(editing.document, at);
  const said = editing.said.get(id);

  return (
    <Row
      id={id}
      label={field.label}
      hint={field.hint}
      required={field.required}
      said={said}
    >
      <Control
        id={id}
        shape={field.shape}
        required={field.required}
        value={value}
        names={editing.names}
        keepEmpty={keepEmpty}
        onChange={(next) => editing.onChange(at, next)}
      />
    </Row>
  );
}

function Control({
  id,
  shape,
  required,
  value,
  names,
  keepEmpty,
  onChange,
}: {
  id: string;
  shape: Shape;
  required?: boolean;
  value: unknown;
  names: (kind: EditableKind) => readonly string[];
  keepEmpty?: boolean;
  onChange: (value: unknown) => void;
}) {
  const written = value === undefined || value === null ? "" : String(value);
  const emptied = keepEmpty ? "" : undefined;

  switch (shape.of) {
    case "text":
    case "longText":
      return (
        <TextControl
          id={id}
          value={written}
          placeholder={shape.of === "text" ? shape.placeholder : undefined}
          long={shape.of === "longText"}
          onChange={(next) => onChange(next === "" ? emptied : next)}
        />
      );
    case "number":
      return <NumberControl id={id} value={written} onChange={onChange} />;
    case "flag":
      return (
        <FlagControl
          id={id}
          value={value === true}
          onChange={(next) => onChange(next ? true : undefined)}
        />
      );
    case "choice":
      return (
        <ChoiceControl
          id={id}
          value={written}
          options={shape.options}
          required={required}
          onChange={onChange}
        />
      );
    case "reference":
      return (
        <ReferenceControl
          id={id}
          value={written}
          names={names(shape.to)}
          onChange={(next) => onChange(next === "" ? emptied : next)}
        />
      );
    case "list":
    case "record":
    case "variants":
      return null;
  }
}

/** A heading for the fields that sit under it, and what was said about them. */
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
          editing.onChange([...at, entries.length], blank(shape.entry))
        }
      >
        <Plus />
        Add {shape.entryLabel}
      </Button>
    </div>
  );
}

/** One entry of a list, which is whatever the entry's shape is. */
function Entry({
  called,
  shape,
  at,
  editing,
}: {
  /** What this entry is called: the list's own noun and its place in it. */
  called: string;
  shape: Shape;
  at: Path;
  editing: Editing;
}) {
  if (shape.of === "record" || shape.of === "variants") {
    return (
      <fieldset className="flex flex-col gap-3 rounded-md border border-border p-3">
        <legend className={cn(TEXT_LABEL, "px-1 font-medium")}>{called}</legend>

        {shape.of === "record" ? (
          <FieldsView fields={shape.fields} at={at} editing={editing} />
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

/** What a new entry of a list starts as, by the shape of an entry. */
function blank(shape: Shape): unknown {
  switch (shape.of) {
    case "list":
      return [];
    case "record":
    case "variants":
      return {};
    case "flag":
      return false;
    case "number":
      return 0;
    case "text":
    case "longText":
    case "choice":
    case "reference":
      return "";
  }
}
