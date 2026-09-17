import { describe, expect, it } from "vitest";

import type { Field, Shape } from "./describe";
import { spell } from "./describe";
import { DESCRIPTIONS } from "./kinds";

function fieldsOf(shape: Shape): readonly Field[] {
  if (shape.of === "record") return shape.fields;
  if (shape.of === "variants") return Object.values(shape.variants).flat();
  if (shape.of === "list") return fieldsOf(shape.entry);
  return [];
}

/** A variant's fields sit beside the choice, so they are read as siblings. */
function flattened(fields: readonly Field[]): readonly Field[] {
  return fields.flatMap((field) =>
    field.shape.of === "variants"
      ? flattened(Object.values(field.shape.variants).flat())
      : [field]
  );
}

/**
 * Every set of fields that sit side by side in one document.
 *
 * A record with variants is several such sets: the fields it always has, plus
 * the fields of whichever variant it is.
 */
function siblings(fields: readonly Field[]): (readonly Field[])[] {
  const always = fields.filter(({ shape }) => shape.of !== "variants");
  const choices = fields.flatMap(({ shape }) =>
    shape.of === "variants"
      ? Object.values(shape.variants).map((variant) => [...always, ...variant])
      : []
  );
  const nested = fields.flatMap(({ shape }) => below(shape));

  return [...(choices.length === 0 ? [always] : choices), ...nested];
}

function below(shape: Shape): (readonly Field[])[] {
  if (shape.of === "record") return siblings(shape.fields);
  if (shape.of === "variants") {
    return Object.values(shape.variants).flatMap(siblings);
  }
  if (shape.of === "list") return below(shape.entry);

  return [];
}

/** Every field in a description, with the path the editor would give it. */
function walk(
  fields: readonly Field[],
  at: (string | number)[] = []
): { path: string; field: Field }[] {
  return flattened(fields).flatMap((field) => {
    const here = [...at, field.name];
    return [{ path: spell(here), field }, ...walk(fieldsOf(field.shape), here)];
  });
}

const ALL = Object.values(DESCRIPTIONS);

describe("the kind descriptions", () => {
  it.each(Object.entries(DESCRIPTIONS))(
    "%s calls itself by the name it is stored under",
    (key, description) => {
      expect(description.kind).toBe(key);
      expect(description.noun).not.toBe("");
    }
  );

  it.each(ALL)("$kind labels every field it offers", (description) => {
    for (const { path, field } of walk(description.fields)) {
      expect(field.label, `unlabelled: ${path}`).toBeTruthy();
    }
  });

  // Two fields of one name write to one property, so the second silently wins.
  it.each(ALL)("$kind names each field of a record once", (description) => {
    for (const group of siblings(description.fields)) {
      const names = group.map((field) => field.name);
      expect(new Set(names).size, `repeated in ${names.join(", ")}`).toBe(
        names.length
      );
    }
  });

  it.each(ALL)("$kind only points at kinds that exist", (description) => {
    for (const { path, field } of walk(description.fields)) {
      if (field.shape.of !== "reference") continue;
      expect(DESCRIPTIONS[field.shape.to], `dangling: ${path}`).toBeDefined();
    }
  });
});

describe("a dataset", () => {
  const dataset = DESCRIPTIONS.datasets;

  it("asks for the fields a record is read by", () => {
    const fields = dataset.fields.find((field) => field.name === "fields");

    expect(fields?.required).toBe(true);
    expect(fields?.shape.of).toBe("list");
  });

  it("offers every type and role a declared field may take", () => {
    const declared = walk(dataset.fields);
    const type = declared.find(({ path }) => path === "fields.type")?.field;
    const role = declared.find(({ path }) => path === "fields.role")?.field;

    expect(type?.shape).toMatchObject({
      options: ["string", "int", "float", "bool", "datetime"],
    });
    expect(role?.shape).toMatchObject({
      options: ["dimension", "measurable", "time"],
    });
  });

  it("leaves the row identity optional, so records stand on their own", () => {
    const identity = dataset.fields.find(
      (field) => field.name === "row_identity"
    );

    expect(identity?.required).toBeFalsy();
  });
});

describe("a metric", () => {
  const metric = DESCRIPTIONS.metrics;

  it("reads one dataset, named from the catalogue", () => {
    const dataset = metric.fields.find((field) => field.name === "dataset");

    expect(dataset?.required).toBe(true);
    expect(dataset?.shape).toEqual({ of: "reference", to: "datasets" });
  });

  // A field may be read by name or counted, but it is always called something.
  it("asks what each read column is called, not which one it reads", () => {
    const read = walk(metric.fields);

    expect(
      read.find(({ path }) => path === "fields.as_name")?.field.required
    ).toBe(true);
    expect(
      read.find(({ path }) => path === "fields.field")?.field.required
    ).toBeFalsy();
  });

  it("leaves the window's field optional, for the dataset's own to serve", () => {
    const time = metric.fields.find((field) => field.name === "time");

    expect(time?.required).toBeFalsy();
  });
});

describe("a widget", () => {
  const widget = DESCRIPTIONS.widgets;
  const type = widget.fields.find((field) => field.name === "type");

  it("chooses a type, and each type asks for what it draws", () => {
    expect(type?.shape.of).toBe("variants");
    // The chosen key is stored, and beside the fields it brings, not under it.
    expect(type?.shape.of === "variants" ? type.shape.recorded : "").toBe(
      "type"
    );
    expect(
      type?.shape.of === "variants" ? Object.keys(type.shape.variants) : []
    ).toEqual(["table", "line", "bar", "area", "stat", "pie"]);
  });

  it("asks every type for the metric behind it", () => {
    const variants = type?.shape.of === "variants" ? type.shape.variants : {};

    for (const [name, fields] of Object.entries(variants)) {
      const metric = fields.find((field) => field.name === "metric");
      expect(metric?.shape, `no metric on ${name}`).toEqual({
        of: "reference",
        to: "metrics",
      });
    }
  });
});

describe("a dashboard", () => {
  const dashboard = DESCRIPTIONS.dashboards;
  const items = dashboard.fields.find((field) => field.name === "items");

  it("holds items that are told apart by what they carry", () => {
    const entry = items?.shape.of === "list" ? items.shape.entry : undefined;

    expect(entry?.of).toBe("variants");
    // Nothing in the item says which variant it is; carrying the field does.
    expect(entry?.of === "variants" ? entry.recorded : "set").toBeUndefined();
    expect(entry?.of === "variants" ? Object.keys(entry.variants) : []).toEqual(
      ["widget", "heading", "text"]
    );
  });

  it("draws widgets by name, from the catalogue", () => {
    const named = walk(dashboard.fields).find(
      ({ path }) => path === "items.widget"
    );

    expect(named?.field.shape).toEqual({ of: "reference", to: "widgets" });
  });
});
