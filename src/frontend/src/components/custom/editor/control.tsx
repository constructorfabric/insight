import type { Describing } from "@/lib/custom/editor/aria";
import type { Path, Shape } from "@/lib/custom/editor/describe";
import { read } from "@/lib/custom/editor/document";
import { called } from "@/lib/custom/editor/exclusive";
import { tableAddress } from "@/lib/custom/editor/source";
import {
  ChoiceControl,
  FlagControl,
  NumberControl,
  ReferenceControl,
  TextControl,
} from "@/components/custom/editor/controls";
import type { Editing } from "@/components/custom/editor/fields-view";

export function Control({
  id,
  shape,
  at,
  required,
  value,
  editing,
  keepEmpty,
  describe,
  onChange,
}: {
  id: string;
  shape: Shape;
  at: Path;
  required?: boolean;
  value: unknown;
  editing: Editing;
  keepEmpty?: boolean;
  describe?: Describing;
  onChange: (value: unknown) => void;
}) {
  const written = shown(value);
  const emptied = keepEmpty ? "" : undefined;

  switch (shape.of) {
    case "text":
    case "longText":
      return (
        <TextControl
          id={id}
          describe={describe}
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
          describe={describe}
          value={value === true}
          onChange={(next) => onChange(next ? true : undefined)}
        />
      );
    case "choice":
      return (
        <ChoiceControl
          id={id}
          describe={describe}
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
          describe={describe}
          value={written}
          names={editing.names(shape.to)}
          onChange={(next) => onChange(next === "" ? emptied : next)}
        />
      );
    case "pick":
      return (
        <ReferenceControl
          id={id}
          describe={describe}
          value={written}
          names={[...offered(shape.from, editing), ...(shape.also ?? [])]}
          onChange={(next) => onChange(next === "" ? emptied : next)}
        />
      );
    case "typed": {
      const type =
        declaredType(shape.declared, at, editing) ??
        read(editing.document, [...at.slice(0, -1), shape.by]);

      return (
        <Control
          id={id}
          describe={describe}
          shape={typedAs(type)}
          at={at}
          required={required}
          value={value}
          editing={editing}
          keepEmpty={keepEmpty}
          onChange={(next) => onChange(type === "bool" ? asBool(next) : next)}
        />
      );
    }
    case "list":
    case "record":
    case "variants":
      return null;
  }
}

function typedAs(type: unknown): Shape {
  if (type === "int" || type === "float") return { of: "number" };
  if (type === "bool") return { of: "choice", options: ["true", "false"] };

  return { of: "text" };
}

type PickSource = Extract<Shape, { of: "pick" }>["from"];

function offered(from: PickSource, editing: Editing): readonly string[] {
  if ("dataset" in from) {
    return declaration(from.dataset, editing).map((field) => field.name);
  }
  if ("catalogue" in from) return editing.tables();
  if ("table" in from) {
    const address = tableAddress(editing.document, from);
    return address ? editing.columns(address) : [];
  }

  return called(read(editing.document, [from.list]), from.property);
}

/** The type the dataset declares for the field a sibling names, if it does. */
function declaredType(
  by: Extract<Shape, { of: "typed" }>["declared"],
  at: Path,
  editing: Editing
): string | undefined {
  if (by === undefined) return undefined;

  const named = read(editing.document, [...at.slice(0, -1), by.named]);
  return declaration(by.dataset, editing).find((field) => field.name === named)
    ?.type;
}

function declaration(rootProperty: string, editing: Editing) {
  const dataset = read(editing.document, [rootProperty]);
  return typeof dataset === "string" ? editing.declared(dataset) : [];
}

function asBool(chosen: unknown): boolean | undefined {
  if (chosen === "true") return true;
  if (chosen === "false") return false;

  return undefined;
}

function shown(value: unknown): string {
  if (value === undefined || value === null) return "";
  if (typeof value === "object") return JSON.stringify(value);

  return String(value);
}
