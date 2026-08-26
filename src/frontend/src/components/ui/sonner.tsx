import {
  Toaster as KitToaster,
  toast as kitToast,
  type ToasterProps,
} from "@gears-frontx/ui-kit";
import { useTranslation } from "react-i18next";

type ToastOptions = {
  description?: string;
  duration?: number;
};

function add(type: "success" | "error", title: string, options?: ToastOptions) {
  return kitToast.add({
    type,
    title,
    description: options?.description,
    timeout: options?.duration,
  });
}

const toast = {
  success: (title: string, options?: ToastOptions) => add("success", title, options),
  error: (title: string, options?: ToastOptions) => add("error", title, options),
};

function Toaster(props: ToasterProps) {
  const { t } = useTranslation();

  return (
    <KitToaster
      label={t("common.a11y.notifications")}
      closeLabel={t("common.actions.close")}
      {...props}
    />
  );
}

export { Toaster, toast };
