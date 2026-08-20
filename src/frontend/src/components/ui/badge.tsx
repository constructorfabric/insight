import { Badge as KitBadge, type BadgeProps as KitBadgeProps } from "@gears-frontx/ui-kit";

type AppVariant =
  | "default"
  | "secondary"
  | "destructive"
  | "outline"
  | "ghost"
  | "link";

// The kit names variants for the status a badge carries; this codebase names
// them for its paint.
const VARIANT: Record<AppVariant, Pick<KitBadgeProps, "variant" | "shape">> = {
  default: { variant: "info", shape: "pill" },
  secondary: { variant: "muted", shape: "pill" },
  destructive: { variant: "danger", shape: "pill" },
  outline: { variant: "muted", shape: "plain" },
  ghost: { variant: "muted", shape: "plain" },
  link: { variant: "info", shape: "plain" },
};

type BadgeProps = Omit<KitBadgeProps, "variant" | "shape"> & {
  variant?: AppVariant;
};

function Badge({ variant = "default", ...props }: BadgeProps) {
  return <KitBadge data-slot="badge" {...VARIANT[variant]} {...props} />;
}

export { Badge };
