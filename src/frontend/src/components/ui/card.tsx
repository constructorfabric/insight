import { useRender } from "@base-ui/react/use-render";
import {
  Card as KitCard,
  CardAction as KitCardAction,
  CardContent as KitCardContent,
  CardDescription as KitCardDescription,
  CardFooter as KitCardFooter,
  CardHeader as KitCardHeader,
  CardTitle as KitCardTitle,
  type CardActionProps,
  type CardContentProps,
  type CardDescriptionProps,
  type CardFooterProps,
  type CardHeaderProps,
  type CardProps as KitCardProps,
  type CardTitleProps,
} from "@gears-frontx/ui-kit";

import { cn } from "@/lib/utils";

type CardSize = "default" | "sm";

type CardProps = KitCardProps & {
  "data-size"?: CardSize;
  render?: useRender.ComponentProps<"div">["render"];
};

// The kit scopes card spacing and part padding to its own class, which this
// element does not carry.
const RENDERED = cn(
  "group/card flex flex-col overflow-hidden rounded-xl bg-card text-sm text-card-foreground",
  "ring-1 ring-foreground/10 gap-(--card-spacing) py-(--card-spacing)",
  "[--card-spacing:var(--space-6)] data-[size=sm]:[--card-spacing:var(--space-4)]",
  "[&_[data-slot=card-header]]:px-(--card-spacing)",
  "[&_[data-slot=card-content]]:px-(--card-spacing)",
  "[&_[data-slot=card-footer]]:px-(--card-spacing)",
);

function RenderedCard({
  render,
  size,
  className,
  ...props
}: CardProps & { render: NonNullable<CardProps["render"]>; size: CardSize }) {
  return useRender({
    render,
    props: {
      "data-slot": "card",
      "data-size": size,
      className: cn(RENDERED, className),
      ...props,
    },
  });
}

function Card({ size, render, ...props }: CardProps) {
  const resolved = size ?? props["data-size"] ?? "default";

  if (render) {
    return <RenderedCard render={render} size={resolved} {...props} />;
  }

  return <KitCard data-slot="card" data-size={resolved} size={resolved} {...props} />;
}

function CardHeader(props: CardHeaderProps) {
  return <KitCardHeader data-slot="card-header" {...props} />;
}

function CardTitle(props: CardTitleProps) {
  return <KitCardTitle data-slot="card-title" {...props} />;
}

function CardDescription(props: CardDescriptionProps) {
  return <KitCardDescription data-slot="card-description" {...props} />;
}

function CardAction(props: CardActionProps) {
  return <KitCardAction data-slot="card-action" {...props} />;
}

function CardContent(props: CardContentProps) {
  return <KitCardContent data-slot="card-content" {...props} />;
}

function CardFooter(props: CardFooterProps) {
  return <KitCardFooter data-slot="card-footer" {...props} />;
}

export {
  Card,
  CardHeader,
  CardFooter,
  CardTitle,
  CardAction,
  CardDescription,
  CardContent,
};
