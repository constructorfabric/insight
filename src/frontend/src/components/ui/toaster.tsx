import {
  Toaster as KitToaster,
  type ToasterProps,
} from "@gears-frontx/ui-kit";
import { useTranslation } from "react-i18next";

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

export { Toaster };
