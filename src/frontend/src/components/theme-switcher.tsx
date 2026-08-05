import { Monitor, Moon, Sun } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useTheme, type Theme } from "@/components/theme-provider";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";

const THEMES: ReadonlyArray<Theme> = ["light", "dark", "system"];

const ICONS: Record<Theme, typeof Sun> = {
  light: Sun,
  dark: Moon,
  system: Monitor,
};

export function ThemeSwitcher() {
  const { t } = useTranslation();
  const { theme, setTheme } = useTheme();
  const CurrentIcon = ICONS[theme];

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <SidebarMenuButton>
                <CurrentIcon className="size-4" />
                <span>{t(`theme.${theme}`)}</span>
              </SidebarMenuButton>
            }
          />
          <DropdownMenuContent side="top" align="start">
            {THEMES.map((value) => {
              const Icon = ICONS[value];
              return (
                <DropdownMenuItem key={value} onClick={() => setTheme(value)}>
                  <Icon className="size-4" />
                  <span>{t(`theme.${value}`)}</span>
                </DropdownMenuItem>
              );
            })}
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
