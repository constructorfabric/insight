/**
 * The screen rides along as the redacted `lastPath` the usage rows carry, so a
 * report is placed without the sender having to describe where they were.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  FEEDBACK_CATEGORIES,
  type FeedbackCategory,
} from "@/api/feedback-client";
import { ConfirmDialog } from "@/components/confirm-dialog";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { toast } from "@/components/ui/sonner";
import { Textarea } from "@/components/ui/textarea";
import { apiErrorReason } from "@/lib/query-console/api-error";
import { useSubmitFeedback } from "@/queries/feedback";
import { APP_NAME, APP_VERSION, currentScreen } from "@/telemetry";

const DEFAULT_CATEGORY: FeedbackCategory = "idea";

export function FeedbackDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const [category, setCategory] = useState<FeedbackCategory>(DEFAULT_CATEGORY);
  const [message, setMessage] = useState("");
  const submit = useSubmitFeedback();

  const close = () => {
    onOpenChange(false);
    setCategory(DEFAULT_CATEGORY);
    setMessage("");
    submit.reset();
  };

  const send = () => {
    submit.mutate(
      {
        category,
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
      <div className="flex flex-col gap-4">
        <div className="flex flex-col gap-2">
          <Label htmlFor="feedback-category">{t("feedback.category")}</Label>
          <Select
            value={category}
            onValueChange={(value) => setCategory(value as FeedbackCategory)}
          >
            <SelectTrigger id="feedback-category" className="w-full">
              <SelectValue>{t(`feedback.categories.${category}`)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {FEEDBACK_CATEGORIES.map((option) => (
                <SelectItem key={option} value={option}>
                  {t(`feedback.categories.${option}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex flex-col gap-2">
          <Label htmlFor="feedback-message">{t("feedback.message")}</Label>
          <Textarea
            id="feedback-message"
            rows={5}
            value={message}
            placeholder={t("feedback.placeholder")}
            onChange={(event) => setMessage(event.target.value)}
          />
        </div>
      </div>
    </ConfirmDialog>
  );
}
