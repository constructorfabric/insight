import { Button as KitButton, type ButtonProps as KitButtonProps } from "@gears-frontx/ui-kit";
import { cva } from "class-variance-authority";

type AppSize =
  | "default"
  | "xs"
  | "sm"
  | "lg"
  | "icon"
  | "icon-xs"
  | "icon-sm"
  | "icon-lg";

const KIT_SIZE: Record<AppSize, NonNullable<KitButtonProps["size"]>> = {
  default: "default",
  xs: "sm",
  sm: "sm",
  lg: "lg",
  icon: "default",
  "icon-xs": "sm",
  "icon-sm": "sm",
  "icon-lg": "lg",
};

const ICON_ONLY: AppSize[] = ["icon", "icon-xs", "icon-sm", "icon-lg"];

type ButtonProps = Omit<KitButtonProps, "size"> & { size?: AppSize };

function Button({ variant = "default", size = "default", ...props }: ButtonProps) {
  return (
    <KitButton
      data-slot="button"
      data-variant={variant}
      data-size={size}
      data-icon-only={ICON_ONLY.includes(size) ? "" : undefined}
      variant={variant}
      size={KIT_SIZE[size]}
      {...props}
    />
  );
}

// Calendar styles react-day-picker's nav slots with class strings; the kit
// exports no class factory.
const buttonVariants = cva("", {
  variants: {
    variant: {
      default: "bg-primary text-primary-foreground hover:bg-primary/80",
      outline: "border border-border bg-background hover:bg-muted",
      secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/80",
      ghost: "hover:bg-muted hover:text-foreground",
      destructive: "bg-destructive/10 text-destructive hover:bg-destructive/20",
      link: "text-primary underline-offset-4 hover:underline",
    },
  },
  defaultVariants: { variant: "default" },
});

export { Button, buttonVariants };
