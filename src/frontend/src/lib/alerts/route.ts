/** The alert id a path names, read from the path the router navigated to. */
export function alertIdFromPath(pathname: string): string {
  const match = pathname.match(/^\/portal\/custom\/alerts\/([^/]+)/);
  if (!match || match[1] === "new") return "";

  return decodeURIComponent(match[1]);
}
