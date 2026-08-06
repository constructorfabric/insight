/**
 * Which paths the portal shell claims.
 *
 * The root layout has to know whether to render the app chrome or hand the
 * whole screen to the portal, but *which* paths are the portal's is portal
 * knowledge — kept here so the route file only asks the question.
 *
 * `/portal` carries the org zones; a person route carries Person and People.
 * Anything else (/metrics, /whats-new, /queries) keeps the app chrome, which an
 * earlier "the portal replaces the app" branch used to swallow.
 */
const PERSON_SHELL = /^\/ic\/[^/]+\/(personal|team)\/?$/;

export function isPortalShellPath(pathname: string): boolean {
  return pathname === "/portal" || PERSON_SHELL.test(pathname);
}
