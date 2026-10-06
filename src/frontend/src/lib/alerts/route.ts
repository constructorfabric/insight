/**
 * The alert id a path names, read from the path the router navigated to.
 *
 * INVARIANT: the router hands the path decoded already; decoding the segment
 * again would throw on one it left as it came.
 */
export function alertIdFromPath(pathname: string): string {
  const match = pathname.match(/^\/portal\/custom\/alerts\/([^/]+)/);
  if (!match || match[1] === "new") return "";

  return match[1];
}
