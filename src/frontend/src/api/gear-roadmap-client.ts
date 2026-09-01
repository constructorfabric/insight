import { AnalyticsApiError } from "@/api/analytics-client";
import { fetchWithAuth } from "@/api/fetch-with-auth";

const BASE =
  (import.meta.env.VITE_API_BASE as string | undefined) ?? "/api/analytics/v1";

/**
 * Where a gear sits against the month window the server chose.
 *
 * `overdue` is its own state, never folded into `backlog`: a milestone that has
 * already passed and work nobody has scheduled are different problems.
 * `unrecognized` keeps a milestone title the server could not read as a month,
 * so an unreadable title stays visible instead of vanishing.
 */
export type GearPlacement =
  | { kind: "slot"; slot: number }
  /** Done: its milestone is history, not a deadline it missed. */
  | { kind: "delivered" }
  | { kind: "overdue"; days: number }
  | { kind: "future" }
  | { kind: "backlog" }
  | { kind: "unrecognized" }
  | { kind: "none" };

/** The board's own commitment vocabulary; `unstated` means the field is unset. */
export type GearCommitment = "committed" | "planned" | "unstated";

/**
 * One gear on the board.
 *
 * Every optional number means "the board carries no value", never zero: an
 * absent `effort_man_days` is an unestimated gear, and an absent
 * `status_percent` is a gear whose ladder value the server could not read.
 */
export interface Gear {
  number: number;
  title: string;
  subsystem?: string | null;
  status_percent?: number | null;
  design_percent?: number | null;
  sdk_percent?: number | null;
  commitment: GearCommitment | string;
  priority?: string | null;
  effort_man_days?: number | null;
  remaining_man_days?: number | null;
  milestone?: string | null;
  /** Month the schedule lands the gear in; absent when nothing is left. */
  forecast?: string | null;
  placement: GearPlacement;
  assignees: string[];
  closed: boolean;
  /** Absent when no configured source claims the gear's repository. */
  issue_url?: string | null;
  assignee_urls?: AssigneeLink[];
}

/** A login with its account page, where a configured source knows one. */
export interface AssigneeLink {
  login: string;
  url?: string | null;
}

export interface GearSpan {
  gear_number: number;
  start: string;
  end: string;
}

/** One schedule lane. A null assignee is a gear nobody owns. */
export interface GearLane {
  assignee?: string | null;
  /** Absent for an unassigned lane, and where no source knows the account. */
  assignee_url?: string | null;
  spans: GearSpan[];
}

export interface GearRoadmap {
  /** The capacity the schedule assumed, so the page can state it. */
  capacity_man_days_per_person: number;
  /** First month of the window, as `YYYY-MM`. */
  window_start: string;
  window_months: number;
  gears: Gear[];
  lanes: GearLane[];
}

/** The column the server orders gears by, and which way. */
export interface GearOrder {
  sort: string;
  direction: "asc" | "desc";
}

/** No order asked for means the server's own, so the request carries none. */
export async function getGearRoadmap(
  order: GearOrder | null,
): Promise<GearRoadmap> {
  const query = order
    ? `?${new URLSearchParams({
        sort: order.sort,
        direction: order.direction,
      }).toString()}`
    : "";
  const res = await fetchWithAuth(`${BASE}/gear-roadmap${query}`);
  if (!res.ok) {
    throw new AnalyticsApiError(res.status, await res.json().catch(() => null));
  }
  return (await res.json()) as GearRoadmap;
}
