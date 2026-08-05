import { useTranslation } from "react-i18next";

import { useAuth } from "@/auth";
import { FullScreenLoading } from "@/components/full-screen-loading";

type AuthGateProps = {
  children: React.ReactNode;
};

/**
 * Renders the cookie/BFF auth state. The redirect into the login flow is owned
 * by the root `beforeLoad` (and the 401 path in `fetchWithAuth`); this gate only
 * withholds the app subtree until a session is confirmed:
 *   - `loading`         → the boot `/auth/me` probe is in flight.
 *   - `unauthenticated` → a full-page redirect to `/auth/login` is in flight,
 *     so keep showing the overlay rather than painting a 401'd tree.
 *   - `authenticated`   → render the app.
 */
export function AuthGate({ children }: AuthGateProps): React.ReactNode {
  const { t } = useTranslation();
  const { status } = useAuth();

  if (status !== "authenticated") {
    return <FullScreenLoading message={t("auth.redirecting")} />;
  }
  return children;
}
