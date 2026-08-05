import { UserRoundSearch } from "lucide-react";
import { useTranslation } from "react-i18next";

import { signIn, useAuth } from "@/auth";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

/**
 * The "viewing as" indicator for a `__override` view-as session
 * (insight#1941): the authenticator minted this session AS another person and
 * `/auth/me` named the real principal (`impersonator_email`). Exiting is just
 * a fresh login as yourself — the session-fixation guard revokes the view-as
 * session at the callback and the IdP hop is silent SSO.
 */
export function ViewAsBanner(): React.ReactElement | null {
  const { t } = useTranslation();
  const { session } = useAuth();

  if (!session?.impersonatorEmail) return null;

  return (
    <Alert variant="warning" className="rounded-none border-0 border-b">
      <UserRoundSearch />
      <AlertTitle>
        {t("view_as_banner.title", { email: session.email })}
      </AlertTitle>
      <AlertDescription className="flex flex-wrap items-center gap-2">
        <span>
          {t("view_as_banner.description", {
            impersonator: session.impersonatorEmail,
          })}
        </span>
        <Button variant="outline" size="sm" onClick={() => signIn("/")}>
          {t("view_as_banner.exit")}
        </Button>
      </AlertDescription>
    </Alert>
  );
}
