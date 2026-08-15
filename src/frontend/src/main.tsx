import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { QueryClientProvider } from "@tanstack/react-query";
import { I18nextProvider } from "react-i18next";

import "./index.css";
import {
  authStore,
  clearAuthErrorAttempts,
  consumeAuthErrorParam,
  consumeOverrideParam,
  loadSession,
  signIn,
  startSessionRefresh,
} from "@/auth";
import { AppErrorBoundary } from "@/components/app-error-boundary";
import { LoginError } from "@/components/login-error";
import { ThemeProvider } from "@/components/theme-provider";
import i18n from "@/i18n";
import { queryClient } from "@/query-client";
import { initSentry } from "@/sentry";
import { recordPageView, startUsageTelemetry } from "@/telemetry";
import { router } from "./router";

async function enableMocking(): Promise<void> {
  if (!import.meta.env.DEV) return;
  if (import.meta.env.VITE_ENABLE_MOCKS !== "true") return;
  const { worker } = await import("@/mocks/browser");
  await worker.start({ onUnhandledRequest: "bypass" });
}

initSentry(router);

// `?__override=<email>` (view-as, insight#1941) bounces straight into the
// login flow — before mocks, session probe, or the router touch anything.
if (!consumeOverrideParam()) bootstrap();

function bootstrap(): void {
  // A failed OIDC callback lands here as `?auth_error=<reason>` (#2032);
  // consumed before the session probe so the router never sees it.
  const authError = consumeAuthErrorParam();
  void enableMocking()
    // Probe the session once (mocks, if enabled, intercept /auth/me) before the
    // router mounts, so the root beforeLoad reads a resolved auth store.
    .then(() => loadSession())
    .then((status) => {
      if (status === "authenticated") {
        // Covers the replayed-callback bounce too: the first callback already
        // set the cookie, so an `auth_error` here is stale.
        clearAuthErrorAttempts();
        // The session is non-sliding — without the refresh driver it dies
        // session_ttl (~10 min) after login regardless of activity (#1854).
        startSessionRefresh();
        startUsageCollection();
        renderApp();
        return;
      }
      if (authError?.autoRetry) {
        // A fresh login fixes the retryable reasons (expired state after a
        // slow IdP round-trip, IdP hiccup); the attempt counter halts a
        // persistent failure on the error screen instead of looping. No-arg
        // signIn: return to the current URL, already stripped of auth_error.
        signIn();
        return;
      }
      if (authError) {
        renderLoginError(authError.code);
        return;
      }
      // Unauthenticated without an auth_error: the root beforeLoad bounces
      // into the login flow.
      renderApp();
    });
}

/// Adoption events (#2573). Fire-and-forget: the instance may have collection
/// off, and nothing here may delay or break the render.
function startUsageCollection(): void {
  const { session } = authStore.getSnapshot();
  if (!session) return;
  void startUsageTelemetry(session).then(() => {
    recordPageView(portalPath(window.location.pathname, new URLSearchParams(window.location.search)));
    router.subscribe("onResolved", ({ toLocation }) => {
      const search = toLocation.search as Record<string, unknown>;
      recordPageView(
        portalPath(
          toLocation.pathname,
          new URLSearchParams(
            Object.entries(search)
              .filter(([, v]) => typeof v === "string")
              .map(([k, v]) => [k, v as string]),
          ),
        ),
      );
    });
  });
}

/**
 * The portal is a single route whose screen lives in `zone`/`item`, so the
 * path alone would report every portal page as one. Only those two are read —
 * the rest of the query carries scope and person ids.
 */
function portalPath(pathname: string, search: URLSearchParams): string {
  const parts = ["zone", "item"]
    .map((key) => search.get(key))
    .filter((value): value is string => Boolean(value));
  return parts.length ? `${pathname}/${parts.join("/")}` : pathname;
}

function renderApp(): void {
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <AppErrorBoundary>
          <ThemeProvider>
            <I18nextProvider i18n={i18n}>
              <RouterProvider router={router} />
            </I18nextProvider>
          </ThemeProvider>
        </AppErrorBoundary>
      </QueryClientProvider>
    </StrictMode>
  );
}

function renderLoginError(code: string): void {
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <ThemeProvider>
        <I18nextProvider i18n={i18n}>
          <LoginError code={code} />
        </I18nextProvider>
      </ThemeProvider>
    </StrictMode>
  );
}
