/**
 * Shared error type for the Analytics API clients (`metric-results-client`,
 * `metric-definitions-client`). Each client owns its own request/parse path;
 * this carries the HTTP status and the parsed problem body across them.
 */
export class AnalyticsApiError extends Error {
  status: number;
  body: unknown;

  constructor(status: number, body: unknown) {
    super(`Analytics API ${status}`);
    this.name = "AnalyticsApiError";
    this.status = status;
    this.body = body;
  }
}
