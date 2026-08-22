/**
 * The screen rides along as the redacted `lastPath` the usage rows carry, so a
 * report is placed without the sender having to describe where they were.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { ConfirmDialog } from "@/components/confirm-dialog";
import { toast } from "@/components/ui/sonner";
import { Textarea } from "@/components/ui/textarea";
import { apiErrorReason } from "@/lib/query-console/api-error";
import { useSubmitFeedback } from "@/queries/feedback";
import { APP_NAME, APP_VERSION, currentScreen } from "@/telemetry";

export function FeedbackDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [message, setMessage] = useState("");
  const submit = useSubmitFeedback();

  const close = () => {
    onOpenChange(false);
    setMessage("");
    submit.reset();
  };

  const send = () => {
    submit.mutate(
      {
        message,
        path: currentScreen(),
        app_name: APP_NAME,
        app_version: APP_VERSION,
      },
      {
        onSuccess: () => {
          toast.success(t("feedback.sent"));
          close();
        },
      },
    );
  };

  return (
    <ConfirmDialog
      open={open}
      onOpenChange={(next) => (next ? onOpenChange(true) : close())}
      title={t("feedback.title")}
      confirmLabel={t("feedback.send")}
      isPending={submit.isPending}
      confirmDisabled={message.trim().length === 0}
      error={
        submit.error ? apiErrorReason(submit.error, t("feedback.failed")) : null
      }
      onConfirm={send}
    >
      <Textarea
        aria-label={t("feedback.message")}
        rows={5}
        value={message}
        placeholder={t("feedback.placeholder")}
        onChange={(event) => setMessage(event.target.value)}
      />
    </ConfirmDialog>
  );
}
