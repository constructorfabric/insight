import { TriangleAlertIcon } from "lucide-react";
import { Trans, useTranslation } from "react-i18next";

import { Alert, AlertDescription, AlertTitle } from "@gears-frontx/ui-kit";

function isVisible(): boolean {
  if (!import.meta.env.DEV) return false;
  if (import.meta.env.VITE_ENABLE_MOCKS !== "true") return false;
  if (import.meta.env.VITE_HIDE_MOCK_BANNER === "true") return false;
  return true;
}

export function MockBanner(): React.ReactElement | null {
  const { t } = useTranslation();

  if (!isVisible()) return null;

  return (
    <Alert className="rounded-none border-0 border-b text-warning">
      <TriangleAlertIcon />
      <AlertTitle>{t("mock_banner.title")}</AlertTitle>
      <AlertDescription>
        <Trans
          i18nKey="mock_banner.description_html"
          components={{ code: <code className="font-mono" /> }}
        />
      </AlertDescription>
    </Alert>
  );
}
