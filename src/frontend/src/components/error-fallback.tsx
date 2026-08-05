import { AlertCircleIcon } from "lucide-react";
import { type FallbackProps } from "react-error-boundary";
import { useTranslation } from "react-i18next";

import {
  Alert,
  AlertDescription,
  AlertTitle,
} from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

export function ErrorFallback({
  error,
  resetErrorBoundary,
}: FallbackProps): React.ReactElement {
  const { t } = useTranslation();
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : "Unknown error";
  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <Alert variant="destructive" className="max-w-md">
        <AlertCircleIcon />
        <AlertTitle>{t("error_boundary.title")}</AlertTitle>
        <AlertDescription>
          <p className="font-mono text-xs">{message}</p>
          <div className="mt-4 flex gap-2">
            <Button size="sm" onClick={resetErrorBoundary}>
              {t("common.actions.try_again")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => window.location.reload()}
            >
              {t("common.actions.reload")}
            </Button>
          </div>
        </AlertDescription>
      </Alert>
    </div>
  );
}
