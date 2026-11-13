export const CONNECTORS_BASE =
  (import.meta.env.VITE_CONNECTORS_BASE as string | undefined) ??
  "/api/connectors/v1";

export class ConnectorsApiError extends Error {
  status: number;
  body: unknown;

  constructor(status: number, body: unknown) {
    super(`Connectors API ${status}`);
    this.name = "ConnectorsApiError";
    this.status = status;
    this.body = body;
  }
}
