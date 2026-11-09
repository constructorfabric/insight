import { MessageSquare, Settings } from "lucide-react";
import { useState, type ReactElement } from "react";
import { useTranslation } from "react-i18next";
import brandSymbol from "@/assets/brand-symbol.svg";

import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import { useFeedbackDialog } from "@/components/feedback-context";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  useSidebar,
} from "@gears-frontx/ui-kit";
import { useRailNav } from "@/lib/portal/use-rail-nav";
import { cn } from "@/lib/utils";

/**
 * The icon rail: the first column of the shell sidebar (`ShellSidebar`), one
 * icon per rail item (`rail-model.ts`). Collapsing the shell sidebar leaves
 * only this column; picking an item opens the pane beside it again.
 */
export function LensRail() {
  const { items, activeItem, selectItem } = useRailNav();
  const { setOpen } = useSidebar();

  return (
    <Sidebar
      collapsible="none"
      data-testid="lens-rail"
      // WORKAROUND: the kit portals its phone drawer out of the provider, so the
      // icon width the provider sets never reaches it; 3rem is the kit's default.
      className="w-[calc(var(--sidebar-width-icon,3rem)+1px)] border-e"
    >
      <SidebarHeader>
        <img
          src={brandSymbol}
          alt="Constructor Fabric"
          className="size-8 shrink-0"
          width={32}
          height={32}
        />
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu>
              {items.map((item) => (
                <SidebarMenuItem key={item.id}>
                  <RailTooltip label={item.label}>
                    <SidebarMenuButton
                      aria-label={item.label}
                      data-planned={item.planned || undefined}
                      isActive={activeItem === item.id}
                      className={cn(item.planned && "text-muted-foreground")}
                      onClick={() => {
                        selectItem(item);
                        setOpen(true);
                      }}
                    >
                      <item.icon />
                      <span>{item.label}</span>
                    </SidebarMenuButton>
                  </RailTooltip>
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <SidebarMenu>
          <FeedbackItem />
          <SettingsItem />
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  );
}

/** Its own rail slot: inside the settings menu nobody found it. */
function FeedbackItem() {
  const { t } = useTranslation();
  const feedback = useFeedbackDialog();
  const { setOpenMobile } = useSidebar();

  if (!feedback) return null;

  const label = t("feedback.nav_label");
  return (
    <SidebarMenuItem>
      <RailTooltip label={label}>
        <SidebarMenuButton
          aria-label={label}
          // The feedback box opens behind the phone drawer.
          onClick={() => {
            feedback.openFeedback();
            setOpenMobile(false);
          }}
        >
          <MessageSquare />
          <span>{label}</span>
        </SidebarMenuButton>
      </RailTooltip>
    </SidebarMenuItem>
  );
}

function SettingsItem() {
  const [open, setOpen] = useState(false);
  const { setOpenMobile } = useSidebar();

  return (
    <SidebarMenuItem>
      <Popover open={open} onOpenChange={setOpen}>
        <RailTooltip label="Settings">
          <PopoverTrigger
            render={
              <SidebarMenuButton aria-label="Settings">
                <Settings />
                <span>Settings</span>
              </SidebarMenuButton>
            }
          />
        </RailTooltip>
        <PopoverContent side="right" align="end" className="w-64 gap-0 p-1">
          <AppSidebarFooter
            showFeedback={false}
            onNavigate={() => {
              setOpen(false);
              setOpenMobile(false);
            }}
          />
        </PopoverContent>
      </Popover>
    </SidebarMenuItem>
  );
}

/**
 * The rail's labels at every width. The kit's own `tooltip` prop shows only
 * while the whole sidebar is collapsed, and the rail is visible open or not.
 */
function RailTooltip({
  label,
  children,
}: {
  label: string;
  children: ReactElement;
}) {
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}
