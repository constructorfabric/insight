export function dashboardIdFromPath(pathname: string): string {
  const match = pathname.match(/^\/dashboards\/([^/]+)/);
  return match ? decodeURIComponent(match[1]) : "";
}
