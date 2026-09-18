/** What a control tells assistive technology about the row it sits in. */
export interface Describing {
  "aria-describedby"?: string;
  "aria-invalid"?: true;
  "aria-required"?: true;
}

export function hintId(id: string): string {
  return `${id}-hint`;
}

export function saidId(id: string): string {
  return `${id}-said`;
}

/**
 * The row's hint and refusal, bound to its control.
 *
 * Text beside a control is read by nobody who cannot see it unless the control
 * names it, so the row's hint and whatever the service said about it are.
 */
export function describing(
  id: string,
  row: { hint?: string; said?: string; required?: boolean }
): Describing {
  const by = [
    row.hint ? hintId(id) : null,
    row.said ? saidId(id) : null,
  ].filter((one): one is string => one !== null);

  return {
    ...(by.length > 0 ? { "aria-describedby": by.join(" ") } : {}),
    ...(row.said ? { "aria-invalid": true as const } : {}),
    ...(row.required ? { "aria-required": true as const } : {}),
  };
}
