/**
 * The screen rides along as the redacted `lastPath` the usage rows carry, so a
 * report is placed without the sender having to describe where they were.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { FEEDBACK_KINDS, type FeedbackKind } from "@/api/feedback-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { toast } from "@/components/ui/sonner";
import { Textarea } from "@/components/ui/textarea";
import {
  ToggleGroup,
  ToggleGroupItem,
} from "@/components/ui/toggle-group";
import { apiErrorReason } from "@/lib/query-console/api-error";
import { useSubmitFeedback } from "@/queries/feedback";
import { APP_NAME, APP_VERSION, currentScreen } from "@/telemetry";

const DEFAULT_KIND: FeedbackKind = "bug";

export function FeedbackDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<FeedbackKind>(DEFAULT_KIND);
  const [message, setMessage] = useState("");
  const submit = useSubmitFeedback();

  const close = () => {
    onOpenChange(false);
    setKind(DEFAULT_KIND);
    setMessage("");
    submit.reset();
  };

  const send = () => {
    submit.mutate(
      {
        kind,
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
      description={t("feedback.description")}
      confirmLabel={t("feedback.send")}
      isPending={submit.isPending}
      confirmDisabled={message.trim().length === 0}
      error={
        submit.error ? apiErrorReason(submit.error, t("feedback.failed")) : null
      }
      onConfirm={send}
    >
      <div className="flex flex-col gap-3">
        <ToggleGroup
          aria-label={t("feedback.kind_label")}
          value={[kind]}
          onValueChange={(values) => {
            const next = Array.isArray(values) ? values[0] : values;
            if (next) setKind(next as FeedbackKind);
          }}
          variant="outline"
          size="sm"
          className="w-full"
        >
          {FEEDBACK_KINDS.map((option) => (
            <ToggleGroupItem key={option} value={option} className="flex-auto">
              {t(`feedback.kinds.${option}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <Textarea
          aria-label={t("feedback.message")}
          rows={5}
          value={message}
          placeholder={t(`feedback.placeholder.${kind}`)}
          onChange={(event) => setMessage(event.target.value)}
        />
      </div>
    </ConfirmDialog>
  );
}
