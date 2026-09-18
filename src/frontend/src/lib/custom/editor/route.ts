import type { EditableKind } from "@/api/custom-client";

import { DESCRIPTIONS } from "./kinds";

/** What an editor's path names, checked against the kinds that exist. */
export function kindFromPath(
  pathname: string,
  under: "new" | "edit"
): EditableKind | undefined {
  const match = pathname.match(new RegExp(`^/portal/custom/${under}/([^/]+)`));
  const named = match ? decodeURIComponent(match[1]) : "";

  return Object.hasOwn(DESCRIPTIONS, named)
    ? (named as EditableKind)
    : undefined;
}

export function nameFromPath(pathname: string): string | undefined {
  const match = pathname.match(/^\/portal\/custom\/edit\/[^/]+\/([^/]+)/);

  return match ? decodeURIComponent(match[1]) : undefined;
}
