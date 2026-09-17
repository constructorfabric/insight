import { ChoiceControl, Row } from "@/components/custom/editor/controls";
import {
  FieldsView,
  type Editing,
} from "@/components/custom/editor/fields-view";
import type { Field, Path, Shape } from "@/lib/custom/editor/describe";
import { spell } from "@/lib/custom/editor/describe";
import { read } from "@/lib/custom/editor/document";

/**
 * A record whose fields are the ones its variant brings.
 *
 * The choice sits beside those fields rather than above them, because that is
 * where the document holds it.
 */
export function Variants({
  field,
  shape,
  at,
  editing,
}: {
  field: Field;
  shape: Extract<Shape, { of: "variants" }>;
  at: Path;
  editing: Editing;
}) {
  const record = asRecord(read(editing.document, at));
  const chosen = variantOf(shape, record);
  const id = spell([...at, shape.recorded ?? "kind"]);

  return (
    <div className="flex flex-col gap-4">
      <Row
        id={id}
        label={field.label}
        hint={field.hint}
        required
        said={editing.said.get(id)}
      >
        <ChoiceControl
          id={id}
          value={chosen}
          options={Object.keys(shape.variants)}
          required
          onChange={(next) =>
            editing.onChange(at, switched(shape, record, next ?? ""))
          }
        />
      </Row>

      {chosen === "" ? null : (
        <FieldsView
          fields={shape.variants[chosen] ?? []}
          at={at}
          editing={editing}
          keepEmpty={shape.recorded === undefined ? [chosen] : undefined}
        />
      )}
    </div>
  );
}

function asRecord(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

/** Which variant a record is, by what it records or by what it carries. */
function variantOf(
  shape: Extract<Shape, { of: "variants" }>,
  record: Record<string, unknown>
): string {
  if (shape.recorded !== undefined) {
    const said = record[shape.recorded];
    return typeof said === "string" ? said : "";
  }

  return (
    Object.keys(shape.variants).find((name) => record[name] !== undefined) ?? ""
  );
}

/**
 * The record with another variant chosen.
 *
 * The outgoing variant's fields go with it: left behind, they would be sent to
 * a service that reads the record as its new variant and refuses the rest.
 */
function switched(
  shape: Extract<Shape, { of: "variants" }>,
  record: Record<string, unknown>,
  to: string
): Record<string, unknown> {
  const variant = new Set(
    Object.values(shape.variants)
      .flat()
      .map((field) => field.name)
  );
  const kept = Object.entries(record).filter(
    ([name]) => !variant.has(name) && name !== shape.recorded
  );

  if (to === "") return Object.fromEntries(kept);

  return shape.recorded === undefined
    ? { ...Object.fromEntries(kept), [to]: "" }
    : { ...Object.fromEntries(kept), [shape.recorded]: to };
}
