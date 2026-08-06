import { HelpCircle, PanelsTopLeft, Wrench } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { useSettings } from "@/hooks/use-settings";
import type { FocusMode } from "@/lib/peers";
import {
  setPortalEnabled,
  setPortalShowPlanned,
  usePortalEnabled,
  usePortalShowPlanned,
} from "@/lib/portal/portal-store";

const FOCUS_MODES: ReadonlyArray<FocusMode> = [
  "critical",
  "rewards",
  "neutral",
  "all",
];

export function SidebarSettings() {
  const { t } = useTranslation();
  const portal = usePortalEnabled();
  const showPlanned = usePortalShowPlanned();
  const { focusMode, showExplanations, setFocusMode, setShowExplanations } =
    useSettings();

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <SidebarMenuButton
          onClick={() => setPortalEnabled(!portal)}
          aria-pressed={portal}
          className="justify-between"
        >
          <span className="flex items-center gap-2">
            <PanelsTopLeft className="size-4" />
            <span>Portal (preview)</span>
          </span>
          <Switch
            checked={portal}
            onCheckedChange={setPortalEnabled}
            size="sm"
            tabIndex={-1}
          />
        </SidebarMenuButton>
      </SidebarMenuItem>
      {/* Only meaningful inside the portal, so it hides with it. Shows the
          zones and views we have scaffolded but not built. */}
      {portal ? (
        <SidebarMenuItem>
          <SidebarMenuButton
            onClick={() => setPortalShowPlanned(!showPlanned)}
            aria-pressed={showPlanned}
            className="justify-between"
          >
            <span className="flex items-center gap-2">
              <Wrench className="size-4" />
              <span>Show planned sections</span>
            </span>
            <Switch
              checked={showPlanned}
              onCheckedChange={setPortalShowPlanned}
              size="sm"
              tabIndex={-1}
            />
          </SidebarMenuButton>
        </SidebarMenuItem>
      ) : null}
      <SidebarMenuItem className="flex flex-col items-stretch gap-1.5 p-1">
        <span className="px-1 text-[10px] font-medium uppercase tracking-wider text-sidebar-foreground/60">
          {t("settings.focus_mode.label")}
        </span>
        <ToggleGroup
          aria-label={t("settings.focus_mode.label")}
          value={[focusMode]}
          onValueChange={(values) => {
            const next = Array.isArray(values) ? values[0] : values;
            if (next) setFocusMode(next as FocusMode);
          }}
          variant="outline"
          size="sm"
          className="w-full"
        >
          {FOCUS_MODES.map((mode) => (
            <ToggleGroupItem key={mode} value={mode} className="flex-1 text-xs">
              {t(`settings.focus_mode.${mode}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </SidebarMenuItem>
      <SidebarMenuItem>
        <SidebarMenuButton
          onClick={() => setShowExplanations(!showExplanations)}
          aria-pressed={showExplanations}
          className="justify-between"
        >
          <span className="flex items-center gap-2">
            <HelpCircle className="size-4" />
            <span>{t("settings.explanations.label")}</span>
          </span>
          <Switch
            checked={showExplanations}
            onCheckedChange={setShowExplanations}
            size="sm"
            tabIndex={-1}
          />
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
