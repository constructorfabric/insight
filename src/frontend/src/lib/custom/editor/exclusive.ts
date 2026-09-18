import { asRecord } from "./variant";

/**
 * The list with one entry marked and every other unmarked.
 *
 * A mark only one entry may carry - a dataset's main date - moves rather than
 * multiplies, so the service is never sent two.
 */
export function alone(list: unknown, index: number, name: string): unknown[] {
  const entries = Array.isArray(list) ? list : [];

  return entries.map((entry, at) => {
    const record = asRecord(entry);
    if (at === index) return { ...record, [name]: true };

    const { [name]: _dropped, ...rest } = record;
    return rest;
  });
}

export function called(list: unknown, property: string): string[] {
  if (!Array.isArray(list)) return [];

  return list
    .map((entry) => asRecord(entry)[property])
    .filter(
      (value): value is string => typeof value === "string" && value !== ""
    );
}
