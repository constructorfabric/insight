import type { EditableKind } from "@/api/custom-client";

import { DESCRIPTIONS } from "./kinds";

/** One literal pattern per editor path, rather than one built per call. */
const KIND_PATTERNS = {
  new: /^\/portal\/custom\/new\/([^/]+)/,
  edit: /^\/portal\/custom\/edit\/([^/]+)/,
} as const;

/** What an editor's path names, checked against the kinds that exist. */
export function kindFromPath(
  pathname: string,
  under: "new" | "edit"
): EditableKind | undefined {
  const match = pathname.match(KIND_PATTERNS[under]);
  const named = match ? decodeURIComponent(match[1]) : "";

  return Object.hasOwn(DESCRIPTIONS, named)
    ? (named as EditableKind)
    : undefined;
}

export function nameFromPath(pathname: string): string | undefined {
  const match = pathname.match(/^\/portal\/custom\/edit\/[^/]+\/([^/]+)/);

  return match ? decodeURIComponent(match[1]) : undefined;
}
