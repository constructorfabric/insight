import type { EditableKind } from "@/api/custom-client";

import { DESCRIPTIONS } from "./kinds";

/**
 * What an editor's path names.
 *
 * Read off the path rather than the router's params, as the rest of the Custom
 * zone does, and checked against the kinds that exist: a path naming something
 * else has no editor to show.
 */
export function kindFromPath(
  pathname: string,
  under: "new" | "edit"
): EditableKind | undefined {
  const match = pathname.match(new RegExp(`^/portal/custom/${under}/([^/]+)`));
  const named = match ? decodeURIComponent(match[1]) : "";

  return named in DESCRIPTIONS ? (named as EditableKind) : undefined;
}

/** The definition an edit path names. */
export function nameFromPath(pathname: string): string | undefined {
  const match = pathname.match(/^\/portal\/custom\/edit\/[^/]+\/([^/]+)/);

  return match ? decodeURIComponent(match[1]) : undefined;
}
