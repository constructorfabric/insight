import {
  Badge as KitBadge,
  type BadgeProps as KitBadgeProps,
} from "@gears-frontx/ui-kit";

function Badge(props: KitBadgeProps) {
  return <KitBadge data-slot="badge" {...props} />;
}

export { Badge };
