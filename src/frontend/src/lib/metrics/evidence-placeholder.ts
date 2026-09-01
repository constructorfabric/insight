import { hashKey, type QueryKey } from "@tanstack/react-query";

/**
 * Whether two evidence reads are the same subject asked a different way.
 *
 * The last key segment carries the order and the needle: re-reading under one
 * of those must keep the rows on screen, or a header click replaces the table
 * — and the search box above it — with a spinner and loses the caret mid-word.
 * Every earlier segment names WHAT is being read, and holding the previous
 * metric's records under the new one's title presents them as its answer.
 */
export function sameEvidenceSubject(
  previous: QueryKey | undefined,
  current: QueryKey
): boolean {
  if (!previous || previous.length !== current.length) return false;
  return hashKey(previous.slice(0, -1)) === hashKey(current.slice(0, -1));
}
