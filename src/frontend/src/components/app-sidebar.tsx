import { useTranslation } from "react-i18next";

import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import { OrgTree } from "@/components/org-tree";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
} from "@/components/ui/sidebar";

export function AppSidebar() {
  const { t } = useTranslation();

  return (
    <Sidebar>
      <SidebarHeader>
        <div className="flex items-center gap-2 px-2 py-1.5">
          <div className="flex size-7 items-center justify-center rounded-md bg-sidebar-primary font-semibold text-sidebar-primary-foreground">
            I
          </div>
          <span className="font-semibold tracking-tight text-sidebar-foreground">
            {t("common.app_name")}
          </span>
        </div>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupContent>
            <OrgTree />
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <AppSidebarFooter />
      </SidebarFooter>
    </Sidebar>
  );
}
