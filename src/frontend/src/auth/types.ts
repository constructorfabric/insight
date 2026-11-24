// Cookie/BFF auth model (NGINX_BFF). The browser holds only the opaque
// `__Host-sid` session cookie; the SPA never sees tokens. Session identity
// comes from the authenticator's `GET /auth/me`.

/**
 * Session lifecycle from the SPA's point of view.
 *   loading         — the initial `/auth/me` probe is in flight.
 *   authenticated   — a live session; `session` is populated.
 *   unauthenticated — no valid session; the app redirects to `/auth/login`.
 */
export type AuthStatus = "loading" | "authenticated" | "unauthenticated";

/** The session summary returned by `GET /auth/me`. */
export type Session = {
  /** Internal person id (UUID) — the gateway JWT `sub`. */
  personId: string;
  /** The person's email — the SPA's person key (org tree, IC routes). */
  email: string;
  /** The signed tenant — one and only one per token (EPIC #1583). */
  tenantId: string;
  /** Access-control roles. */
  roles: string[];
  /**
   * CSRF token bound to the session (issued at login, echoed by `/auth/me`).
   * Sent as `X-CSRF-Token` on state-changing `/auth/*` requests — the
   * authenticator fails closed without it (NGINX_BFF step 10.5).
   */
  csrfToken: string;
  /**
   * Unix seconds when the session dies unless `POST /auth/refresh` extends it.
   * The session is non-sliding: activity never moves this (PRD 5.3).
   */
  expiresAt: number;
  /**
   * Server-supplied unix seconds when the SPA should call `POST /auth/refresh`
   * (`expires_at − safety_margin ± jitter`, re-jittered on every response).
   * Always schedule from this value — never client-computed (PRD 5.4).
   */
  refreshAt: number;
  /**
   * The real principal behind a `__override` view-as session (insight#1941):
   * the authenticator mints the session AS another person and names who is
   * actually signed in here. `null` on normal sessions; non-null drives the
   * "viewing as" banner.
   */
  impersonatorEmail: string | null;
};

export type AuthSnapshot = {
  status: AuthStatus;
  session: Session | null;
};
