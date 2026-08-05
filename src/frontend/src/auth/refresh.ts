import { authStore } from "./auth-store";
import { loadSession } from "./session";
import { signIn } from "./use-auth";

// SPA driver for `POST /auth/refresh` (PRD 5.4 "SPA contract", insight#1854).
//
// The session is non-sliding: only an explicit refresh extends it, so without
// this driver every user is logged out `session_ttl` (~10 min) after login.
// One tab — the leader — owns the refresh timer; every successful response
// carries fresh server-supplied `{expires_at, refresh_at}` and the next timer
// is always scheduled from that value, never client-computed (the server
// jitters `refresh_at` deliberately).
//
// Cross-tab coordination (PRD: "single leader via BroadcastChannel,
// localStorage fallback"):
//   - Leadership is a short-lived lease in localStorage, renewed by heartbeat
//     and stolen once stale — this survives crashed/closed leaders in every
//     browser, with or without BroadcastChannel.
//   - Refresh results fan out over BroadcastChannel; where it is unavailable
//     the same message is written to localStorage so followers get a
//     `storage` event instead.
// Races are tolerated, not prevented: the backend's rotation grace + CAS make
// a double refresh harmless, so the election only has to be *mostly* single.

const LEASE_KEY = "insight.auth.refresh.leader";
const FALLBACK_MSG_KEY = "insight.auth.refresh.msg";
const CHANNEL_NAME = "insight.auth.refresh";

/** Lease older than this is a dead leader; any tab may steal it. */
const LEASE_TTL_MS = 15_000;
/** Leader heartbeat / follower staleness-check cadence. */
const HEARTBEAT_MS = 5_000;
/** Retry delay for transient refresh failures (network, 5xx). */
const RETRY_MS = 15_000;
/** Retry delay for 429 when the response carries no usable hint. */
const RATE_LIMIT_RETRY_MS = 10_000;
/** Abort a hung POST — an unsettled promise would wedge the driver (and the
 *  lease renewal) forever in a single-tab session. */
const REQUEST_TIMEOUT_MS = 10_000;

type RefreshMessage =
  | { kind: "refreshed"; expiresAt: number; refreshAt: number }
  | { kind: "expired" };

type Lease = { id: string; ts: number };

function readLease(): Lease | null {
  try {
    const raw = localStorage.getItem(LEASE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Lease>;
    if (typeof parsed.id !== "string" || typeof parsed.ts !== "number") return null;
    return { id: parsed.id, ts: parsed.ts };
  } catch {
    return null; // corrupt/blocked storage → behave as if no leader
  }
}

class RefreshDriver {
  private readonly tabId = crypto.randomUUID();
  private channel: BroadcastChannel | null = null;
  private heartbeat: ReturnType<typeof setInterval> | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private refreshing = false;
  private stopped = false;
  private readonly cleanups: Array<() => void> = [];

  start(): void {
    if (typeof BroadcastChannel !== "undefined") {
      this.channel = new BroadcastChannel(CHANNEL_NAME);
      this.channel.onmessage = (e: MessageEvent) => {
        this.onMessage(e.data);
      };
    }
    this.listen(window, "storage", (e) => {
      const ev = e as StorageEvent;
      // Fallback fan-out only — with BroadcastChannel available the message
      // never lands in localStorage.
      if (ev.key !== FALLBACK_MSG_KEY || !ev.newValue) return;
      try {
        this.onMessage(JSON.parse(ev.newValue));
      } catch {
        // ignore corrupt cross-tab payloads
      }
    });
    // Timers do not fire while a laptop sleeps or a tab is frozen — on wake,
    // re-evaluate: an overdue refresh fires immediately, a dead leader gets
    // replaced without waiting for the follower heartbeat.
    this.listen(document, "visibilitychange", () => {
      if (document.visibilityState === "visible") this.tick();
    });
    // Let a follower take over promptly instead of waiting out the lease.
    this.listen(window, "pagehide", () => {
      this.releaseLease();
    });
    // signOut() (or any path that clears the store) must stop the driver —
    // a timer surviving logout would 401 and bounce an already-left user.
    this.cleanups.push(
      authStore.subscribe(() => {
        if (authStore.getSnapshot().status === "unauthenticated") this.stop();
      }),
    );

    this.heartbeat = setInterval(() => {
      this.tick();
    }, HEARTBEAT_MS);
    this.tick();
  }

  stop(): void {
    if (this.stopped) return;
    this.stopped = true;
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.clearTimer();
    this.releaseLease();
    this.channel?.close();
    this.channel = null;
    for (const undo of this.cleanups.splice(0)) undo();
  }

  /**
   * One coordination beat: renew or steal the lease, and make sure the leader
   * has a timer armed for the store's current `refreshAt`. Followers keep no
   * timer at all — they inherit the schedule the moment they take the lease.
   */
  private tick(): void {
    if (this.stopped || this.refreshing) return;
    if (!this.tryLead()) {
      this.clearTimer();
      return;
    }
    const session = authStore.getSnapshot().session;
    if (!session || session.refreshAt <= 0) return; // pre-driver backend: never schedule
    if (this.timer !== null) return;
    const delayMs = Math.max(0, session.refreshAt * 1000 - Date.now());
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.refresh();
    }, delayMs);
  }

  /** Renew our lease, or steal an absent/stale one. Losing it is fine — the
   *  backend absorbs the brief two-leaders window. */
  private tryLead(): boolean {
    const lease = readLease();
    if (lease && lease.id !== this.tabId && Date.now() - lease.ts <= LEASE_TTL_MS) {
      return false;
    }
    try {
      localStorage.setItem(LEASE_KEY, JSON.stringify({ id: this.tabId, ts: Date.now() }));
    } catch {
      // Storage blocked (private mode quota etc.) — refresh anyway; the
      // backend tolerates one refresher per tab, just noisier than intended.
    }
    return true;
  }

  private releaseLease(): void {
    try {
      if (readLease()?.id === this.tabId) localStorage.removeItem(LEASE_KEY);
    } catch {
      // ignore — the lease then simply ages out
    }
  }

  private clearTimer(): void {
    if (this.timer !== null) {
      clearTimeout(this.timer);
      this.timer = null;
    }
  }

  private async refresh(): Promise<void> {
    if (this.stopped || this.refreshing) return;
    this.refreshing = true;
    try {
      const csrfToken = authStore.getSnapshot().session?.csrfToken ?? "";
      const res = await fetch("/auth/refresh", {
        method: "POST",
        credentials: "include",
        headers: {
          Accept: "application/json",
          ...(csrfToken ? { "X-CSRF-Token": csrfToken } : {}),
        },
        signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
      });
      // signOut() can land while the request is in flight; a stale result
      // must not be applied or fanned out after the store was cleared.
      if (this.stopped) return;

      if (res.status === 401) {
        // Past the rotation grace or the absolute cap — the session is gone
        // and the cookie already cleared server-side. Every tab goes back
        // through login.
        this.broadcast({ kind: "expired" });
        this.expire();
        return;
      }
      if (res.status === 429) {
        this.retryIn(retryAfterMs(res) ?? RATE_LIMIT_RETRY_MS);
        return;
      }
      if (res.status === 403) {
        // CSRF verification failed — our stored token is somehow stale.
        // Re-probe /auth/me: it re-primes csrf_token and both timestamps.
        if ((await loadSession()) !== "authenticated") {
          // loadSession failed closed; the store flip already stopped the
          // driver via our subscription. Fan out the expiry like the 401
          // path — broadcast() falls through to the localStorage transport
          // now that stop() closed the channel, and every tab listens to
          // storage events regardless of its own transport. Then bounce to
          // login, or the user is stranded on AuthGate's overlay with no
          // redirect in flight.
          this.broadcast({ kind: "expired" });
          this.expire();
          return;
        }
        const refreshAt = authStore.getSnapshot().session?.refreshAt ?? 0;
        if (refreshAt <= 0) {
          this.refreshing = false;
          return;
        }
        // Floor the retry: under a persistent 403 (proxy stripping the
        // header, CSRF regression) the re-primed refresh_at is already due,
        // and rescheduling from it directly would hammer /auth/refresh +
        // /auth/me in a zero-delay loop.
        this.retryIn(Math.max(RETRY_MS, refreshAt * 1000 - Date.now()));
        return;
      }
      if (!res.ok) {
        this.retryIn(RETRY_MS);
        return;
      }

      const body = (await res.json()) as { expires_at?: number; refresh_at?: number };
      if (typeof body.expires_at !== "number" || typeof body.refresh_at !== "number") {
        this.retryIn(RETRY_MS);
        return;
      }
      this.apply(body.expires_at, body.refresh_at);
      this.broadcast({ kind: "refreshed", expiresAt: body.expires_at, refreshAt: body.refresh_at });
      this.refreshing = false;
      this.tick();
    } catch {
      // Network error — retry; refresh_at leaves ≥30 s of session life, and
      // once the session dies the retry's 401 lands on the expire path.
      this.retryIn(RETRY_MS);
    }
  }

  private retryIn(delayMs: number): void {
    this.refreshing = false;
    if (this.stopped) return;
    this.clearTimer();
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.refresh();
    }, delayMs);
  }

  /** Fold fresh `{expires_at, refresh_at}` into the store, keeping identity. */
  private apply(expiresAt: number, refreshAt: number): void {
    const session = authStore.getSnapshot().session;
    if (session) authStore.setAuthenticated({ ...session, expiresAt, refreshAt });
  }

  private expire(): void {
    // stop() first: setUnauthenticated() triggers our own store subscription,
    // and the redirect must not race a live timer.
    this.stop();
    authStore.setUnauthenticated();
    signIn();
  }

  private onMessage(raw: unknown): void {
    // The channel is same-origin, but still shape-check: a newer deploy in
    // another tab may speak a newer dialect, and only an explicit, well-formed
    // message may move auth state — never "anything unrecognized".
    if (this.stopped || typeof raw !== "object" || raw === null) return;
    const msg = raw as Partial<RefreshMessage>;
    if (
      msg.kind === "refreshed" &&
      typeof msg.expiresAt === "number" &&
      typeof msg.refreshAt === "number"
    ) {
      this.apply(msg.expiresAt, msg.refreshAt);
      // A follower stores the new schedule; if it is (or becomes) the leader,
      // re-arm from the fresh refresh_at.
      this.clearTimer();
      this.tick();
      return;
    }
    if (msg.kind === "expired") this.expire();
  }

  private broadcast(msg: RefreshMessage): void {
    if (this.channel) {
      this.channel.postMessage(msg);
      return;
    }
    try {
      // storage events only fire on *other* tabs, and only on value change —
      // the nonce makes consecutive identical messages distinct.
      localStorage.setItem(
        FALLBACK_MSG_KEY,
        JSON.stringify({ ...msg, nonce: crypto.randomUUID() }),
      );
    } catch {
      // ignore — followers will still 401→login on their own next API call
    }
  }

  private listen(
    target: Window | Document,
    type: string,
    handler: (e: Event) => void,
  ): void {
    target.addEventListener(type, handler);
    this.cleanups.push(() => {
      target.removeEventListener(type, handler);
    });
  }
}

/** `Retry-After: <seconds>` if the 429 carries one (delta-seconds form only). */
function retryAfterMs(res: Response): number | null {
  const raw = res.headers?.get?.("Retry-After");
  if (!raw) return null;
  const seconds = Number(raw);
  return Number.isFinite(seconds) && seconds > 0 ? seconds * 1000 : null;
}

/**
 * Start the cross-tab session-refresh driver. Call once at boot, after
 * `loadSession()` resolved to `authenticated` (main.tsx). Returns a stop
 * function (used by tests; the app runs the driver for the page's lifetime).
 */
export function startSessionRefresh(): () => void {
  const driver = new RefreshDriver();
  driver.start();
  return () => {
    driver.stop();
  };
}
