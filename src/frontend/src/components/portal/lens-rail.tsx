import { ChevronsLeft, ChevronsRight, MessageSquare, Settings } from "lucide-react";
import { useState, type ReactElement } from "react";
import { useTranslation } from "react-i18next";
import brandSymbol from "@/assets/brand-symbol.svg";

import { AppSidebarFooter } from "@/components/app-sidebar-footer";
import { useFeedbackDialog } from "@/components/feedback-context";
import {
  Button,
  Popover,
  PopoverContent,
  PopoverTrigger,
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
import {
  setPortalRailLabels,
  usePortalRailLabels,
} from "@/lib/portal/portal-store";
import { useRailNav } from "@/lib/portal/use-rail-nav";
import { cn } from "@/lib/utils";

/**
 * The rail: the first column of the shell sidebar (`ShellSidebar`), one row
 * per rail item (`rail-model.ts`), icons only unless the reader asks for the
 * names beside them. Collapsing the shell sidebar leaves only this column, as
 * icons; picking an item opens the pane beside it again.
 */
export function LensRail() {
  return (
    <Sidebar
      collapsible="none"
      data-testid="lens-rail"
      className="w-(--rail-width) border-e transition-[width] duration-200 ease-linear motion-reduce:transition-none"
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
        <RailItems />
      </SidebarContent>
      <SidebarFooter>
        <RailFooterItems />
        <SidebarMenu>
          <LabelsToggle />
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  );
}

/** Whether the rail's rows show their names right now: always in the phone drawer. */
function useRailLabelsShown(): boolean {
  const labels = usePortalRailLabels();
  const { isMobile, state } = useSidebar();
  return isMobile || (labels && state === "expanded");
}

/** A row's name, faded in once the rail has finished widening. */
function RailName({ children }: { children: string }) {
  const shown = useRailLabelsShown();
  return (
    <span
      className={cn(
        "transition-opacity duration-150 motion-reduce:transition-none",
        shown ? "opacity-100 delay-200" : "opacity-0"
      )}
    >
      {children}
    </span>
  );
}

/**
 * Shows or hides the rail's names: an icon with a tooltip either way, at the
 * rail's end edge. Collapsed to icons, there is no room for names.
 */
function LabelsToggle() {
  const { t } = useTranslation();
  const labels = usePortalRailLabels();
  const { state } = useSidebar();

  if (state === "collapsed") return null;

  const label = labels
    ? t("common.a11y.collapse_navigation")
    : t("common.a11y.expand_navigation");
  const Icon = labels ? ChevronsLeft : ChevronsRight;
  return (
    <SidebarMenuItem className="flex justify-end">
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="ghost"
              size="sm"
              icon={<Icon />}
              aria-label={label}
              onClick={() => setPortalRailLabels(!labels)}
            />
          }
        />
        <TooltipContent side="right">{label}</TooltipContent>
      </Tooltip>
    </SidebarMenuItem>
  );
}

/** One row per rail item: icons in the rail, labelled rows in the phone drawer. */
export function RailItems() {
  const { items, activeItem, selectItem } = useRailNav();
  const { setOpen } = useSidebar();

  return (
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
                  <RailName>{item.label}</RailName>
                </SidebarMenuButton>
              </RailTooltip>
            </SidebarMenuItem>
          ))}
        </SidebarMenu>
      </SidebarGroupContent>
    </SidebarGroup>
  );
}

export function RailFooterItems() {
  return (
    <SidebarMenu>
      <FeedbackItem />
      <SettingsItem />
    </SidebarMenu>
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
          <RailName>{label}</RailName>
        </SidebarMenuButton>
      </RailTooltip>
    </SidebarMenuItem>
  );
}

function SettingsItem() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const { isMobile, setOpenMobile } = useSidebar();
  const label = t("settings.nav_label");

  return (
    <SidebarMenuItem>
      <Popover open={open} onOpenChange={setOpen}>
        <RailTooltip label={label}>
          <PopoverTrigger
            render={
              <SidebarMenuButton aria-label={label}>
                <Settings />
                <RailName>{label}</RailName>
              </SidebarMenuButton>
            }
          />
        </RailTooltip>
        <PopoverContent
          side={isMobile ? "top" : "right"}
          align={isMobile ? "start" : "end"}
          className="w-64 gap-0 p-1"
        >
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
 * The rail's labels while it shows icons only, open or collapsed. The kit's own
 * `tooltip` prop shows only while the whole sidebar is collapsed. Rows that
 * carry their names, in the phone drawer or a labelled rail, get none.
 */
function RailTooltip({
  label,
  children,
}: {
  label: string;
  children: ReactElement;
}) {
  const labelsShown = useRailLabelsShown();
  if (labelsShown) return children;

  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}
