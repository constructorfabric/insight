import { Skeleton as KitSkeleton, type SkeletonProps } from "@gears-frontx/ui-kit";

function Skeleton(props: SkeletonProps) {
  return <KitSkeleton data-slot="skeleton" {...props} />;
}

export { Skeleton };
