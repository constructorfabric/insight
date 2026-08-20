import { Badge as KitBadge, type BadgeProps as KitBadgeProps } from "@gears-frontx/ui-kit";

type AppVariant =
  | "default"
  | "secondary"
  | "destructive"
  | "outline"
  | "ghost"
  | "link";

// The kit names badge variants for the status they carry; this codebase names
// them for their paint. Nothing here is a colour decision — every entry is the
// nearest kit semantic for the paint the call site already asked for.
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
