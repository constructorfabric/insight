import {
  Empty as KitEmpty,
  EmptyActions as KitEmptyActions,
  EmptyContent as KitEmptyContent,
  EmptyDescription as KitEmptyDescription,
  EmptyHeader as KitEmptyHeader,
  EmptyMedia as KitEmptyMedia,
  EmptyTitle as KitEmptyTitle,
  type EmptyActionsProps,
  type EmptyContentProps,
  type EmptyDescriptionProps,
  type EmptyHeaderProps,
  type EmptyMediaProps,
  type EmptyProps,
  type EmptyTitleProps,
} from "@gears-frontx/ui-kit";

import { cn } from "@/lib/utils";

function Empty({ className, ...props }: EmptyProps) {
  return (
    <KitEmpty
      data-slot="empty"
      className={cn("mx-auto", className)}
      {...props}
    />
  );
}

function EmptyHeader(props: EmptyHeaderProps) {
  return <KitEmptyHeader data-slot="empty-header" {...props} />;
}

function EmptyMedia(props: EmptyMediaProps) {
  return <KitEmptyMedia data-slot="empty-icon" {...props} />;
}

function EmptyTitle(props: EmptyTitleProps) {
  return <KitEmptyTitle data-slot="empty-title" {...props} />;
}

function EmptyDescription(props: EmptyDescriptionProps) {
  return <KitEmptyDescription data-slot="empty-description" {...props} />;
}

function EmptyContent(props: EmptyContentProps) {
  return <KitEmptyContent data-slot="empty-content" {...props} />;
}

function EmptyActions(props: EmptyActionsProps) {
  return <KitEmptyActions data-slot="empty-actions" {...props} />;
}

export {
  Empty,
  EmptyActions,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
};
