import { Link, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

export type IcViewToggleProps = {
  person: string;
  hasReports: boolean;
};

type View = "personal" | "team";

function viewFromPath(pathname: string): View {
  if (pathname.includes("/team")) return "team";
  return "personal";
}

export function IcViewToggle({ person, hasReports }: IcViewToggleProps) {
  const { t } = useTranslation();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const active = viewFromPath(pathname);

  if (!hasReports) return null;

  return (
    <ToggleGroup value={[active]} variant="outline" size="default">
      <ToggleGroupItem
        value="personal"
        nativeButton={false}
        render={
          <Link
            to="/ic/$person/personal"
            params={{ person }}
            resetScroll={false}
          />
        }
      >
        {t("ic_route.personal_view_label")}
      </ToggleGroupItem>
      <ToggleGroupItem
        value="team"
        nativeButton={false}
        render={
          <Link
            to="/ic/$person/team"
            params={{ person }}
            resetScroll={false}
          />
        }
      >
        {t("ic_route.team_view_label")}
      </ToggleGroupItem>
    </ToggleGroup>
  );
}
