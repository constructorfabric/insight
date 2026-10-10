import { IsolatedDot } from "../chrome";

export const CURVE = {
  type: "monotone",
  strokeWidth: 2.5,
  dot: IsolatedDot,
  activeDot: { r: 4 },
  isAnimationActive: false,
} as const;
