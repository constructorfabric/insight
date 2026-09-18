import { ChoiceControl, Row } from "@/components/custom/editor/controls";
import {
  FieldsView,
  type Editing,
} from "@/components/custom/editor/fields-view";
import type { Field, Path } from "@/lib/custom/editor/describe";
import { describing } from "@/lib/custom/editor/aria";
import { spell } from "@/lib/custom/editor/describe";
import { read } from "@/lib/custom/editor/document";
import {
  asRecord,
  switched,
  variantOf,
  type VariantsShape,
} from "@/lib/custom/editor/variant";

export function Variants({
  field,
  shape,
  at,
  editing,
}: {
  field: Field;
  shape: VariantsShape;
  at: Path;
  editing: Editing;
}) {
  const record = asRecord(read(editing.document, at));
  const chosen = variantOf(shape, record);
  const id = spell([...at, shape.recorded ?? "kind"]);
  const said = editing.said.get(id);

  return (
    <div className="flex flex-col gap-4">
      <Row
        id={id}
        label={field.label}
        property={
          shape.recorded?.toLowerCase() === field.label.toLowerCase()
            ? undefined
            : shape.recorded
        }
        hint={field.hint}
        required
        said={said}
      >
        <ChoiceControl
          id={id}
          describe={describing(id, { hint: field.hint, said, required: true })}
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
