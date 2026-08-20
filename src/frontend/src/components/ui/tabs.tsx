import {
  Tabs as KitTabs,
  TabsContent as KitTabsContent,
  TabsList as KitTabsList,
  TabsTrigger as KitTabsTrigger,
  type TabsContentProps,
  type TabsListProps,
  type TabsProps,
  type TabsTriggerProps,
} from "@gears-frontx/ui-kit";

function Tabs({ orientation = "horizontal", ...props }: TabsProps) {
  return (
    <KitTabs
      data-slot="tabs"
      data-orientation={orientation}
      orientation={orientation}
      {...props}
    />
  );
}

function TabsList({ variant = "default", ...props }: TabsListProps) {
  return (
    <KitTabsList
      data-slot="tabs-list"
      data-variant={variant}
      variant={variant}
      {...props}
    />
  );
}

function TabsTrigger(props: TabsTriggerProps) {
  return <KitTabsTrigger data-slot="tabs-trigger" {...props} />;
}

function TabsContent(props: TabsContentProps) {
  return <KitTabsContent data-slot="tabs-content" {...props} />;
}

export { Tabs, TabsList, TabsTrigger, TabsContent };
