import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";

import type {
  ConnectorHealthSummary,
  ScheduleRead,
} from "@/api/connector-health-client";
import { AppTooltips } from "@/test/tooltip";
import { pickPreset } from "./connector-schedule-pane.test/preset";

const mocks = vi.hoisted(() => ({
  health: undefined as ConnectorHealthSummary | undefined,
  schedule: undefined as ScheduleRead | undefined,
}));
const NOW = "2026-01-15T12:00:00Z";
const instance = { connector: "example-tracker", source_id: "main" };

vi.mock("@/queries/connector-health", () => ({
  useConnectorHealth: () => ({ data: mocks.health }),
  useConnectorSchedule: () => ({ data: mocks.schedule }),
  useWriteConnectorSchedule: () => ({ mutateAsync: vi.fn() }),
  useConnectorSyncs: () => ({
    data: {
      ...instance,
      window: 50,
      syncs: Array.from({ length: 50 }, (_, index) => ({
        job_id: `example-run-${index}`,
        status: "succeeded",
        started_at: NOW,
        duration_ms: 1_000,
        records_reported: index,
      })),
    },
  }),
}));
vi.mock("@/queries/identity-me", () => ({
  useMe: () => ({ data: undefined }),
}));
vi.mock("@/queries/connector-schedule-author", () => ({
  useConnectorScheduleAuthor: () => "another admin",
}));

import { ConnectorHealthPane } from "./connector-health";

it("bounds expanded schedules, the editor and all history rows without widening the main table", async () => {
  const live = {
    crons: ["0 3 * * *"],
    timezone: "UTC",
    suspended: false,
    observed_at: NOW,
  };
  mocks.health = {
    as_of: NOW,
    checked_at: NOW,
    typical_read_interval_ms: 900_000,
    history_available: true,
    connectors: [{ ...instance, configured: true, schedule: live }],
  };
  mocks.schedule = {
    etag: '"2"',
    schedule: {
      ...instance,
      revision: 2,
      live,
      requested: null,
      status: "inherited",
    },
  };
  render(
    <AppTooltips>
      <ConnectorHealthPane />
    </AppTooltips>
  );
  const table = screen.getByRole("table");
  expect(table.parentElement).toHaveClass("@container");
  expect(table.parentElement!.parentElement).toHaveClass(
    "max-w-full",
    "min-w-0",
    "overflow-x-auto"
  );
  expect(
    within(table).getByRole("columnheader", { name: "Next sync" })
  ).toHaveClass("px-3", "sm:px-4", "whitespace-normal");
  const row = screen.getByRole("row", { expanded: false });
  expect(within(row).getAllByRole("cell")[2]).toHaveClass("px-3", "sm:px-4");
  await userEvent.click(row);

  const cell = document.getElementById(row.getAttribute("aria-controls")!)!;
  const boundary = cell.firstElementChild!;
  const panel = boundary.firstElementChild!;
  function expectBounds() {
    expect(cell).toHaveClass("px-4", "whitespace-normal");
    expect(boundary).toHaveClass("min-w-0", "[contain:inline-size]");
    expect(panel).toHaveClass(
      "sticky",
      "left-4",
      "w-[calc(100cqw-2rem)]",
      "max-w-full",
      "min-w-0",
      "grid-cols-1",
      "[overflow-wrap:anywhere]",
      "whitespace-normal"
    );
    expect(panel).not.toHaveClass("w-max", "min-w-max");
    for (const card of panel.children) expect(card).toHaveClass("min-w-0");
  }
  expectBounds();
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  await pickPreset("custom");
  const input = screen.getByRole("textbox", { name: "Cron expression 1" });
  expect(input).toHaveClass("max-w-full", "min-w-0");
  expect(input.closest("form")).toHaveClass(
    "min-w-0",
    "whitespace-normal",
    "[overflow-wrap:anywhere]"
  );
  expectBounds();
  await userEvent.click(screen.getByRole("button", { name: "Show all 50" }));
  const history = screen.getByRole("table", { name: "Recent syncs" });
  expect(within(history).getAllByRole("row")).toHaveLength(51);
  expect(history).toHaveClass("w-full", "max-w-full", "table-fixed");
  expect(history.parentElement).toHaveClass(
    "min-w-0",
    "max-w-full",
    "whitespace-normal",
    "[overflow-wrap:anywhere]"
  );
  const strip = screen.getByRole("list", {
    name: "Recorded sync statuses, newest on the right",
  });
  expect(strip).toHaveClass("flex-wrap", "max-w-full", "min-w-0");
  expect(strip.parentElement).toHaveClass("flex-wrap", "max-w-full", "min-w-0");
  expect(history.querySelector("thead")).toHaveClass(
    "whitespace-normal",
    "[overflow-wrap:anywhere]"
  );
  expectBounds();
});
