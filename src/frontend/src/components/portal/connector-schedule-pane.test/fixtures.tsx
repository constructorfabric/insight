import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, vi } from "vitest";
import type {
  ConnectorHealth,
  ScheduleRead,
} from "@/api/connector-health-client";

const mocks = vi.hoisted(() => ({
  query: {
    data: undefined as ScheduleRead | undefined,
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  },
  save: vi.fn(),
}));
vi.mock("@/queries/connector-health", () => ({
  useConnectorSchedule: () => mocks.query,
  useWriteConnectorSchedule: () => ({ mutateAsync: mocks.save }),
}));
vi.mock("@/queries/identity-me", () => ({
  useMe: () => ({ data: { person_id: "example-admin" } }),
}));

import { ConnectorSchedulePane } from "../connector-schedule-pane";
import { AppTooltips } from "@/test/tooltip";

export const row: ConnectorHealth = {
  connector: "example-tracker",
  source_id: "main",
  configured: true,
  schedule: {
    crons: ["0 3 * * *"],
    timezone: "UTC",
    origin: "default",
    suspended: false,
  },
};
export const NOW = "2026-01-15T12:00:00Z";

export function read(): ScheduleRead {
  return {
    etag: '"2"',
    schedule: {
      connector: row.connector,
      source_id: "main",
      revision: 2,
      applied: {
        ...row.schedule!,
        origin: "default",
        revision: 1,
        observed_at: NOW,
      },
      requested: {
        crons: ["0 6 * * *"],
        updated_at: NOW,
        updated_by: "example-admin",
      },
      status: "pending",
    },
  };
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  mocks.query.data = read();
  mocks.query.isError = false;
  mocks.query.isPending = false;
  mocks.query.refetch.mockReset();
  mocks.save.mockReset();
});

afterEach(() => vi.useRealTimers());

export function pane() {
  return (
    <AppTooltips>
      <ConnectorSchedulePane row={row} polling={false} />
    </AppTooltips>
  );
}

export async function editCustom(text = "not a cron") {
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  await userEvent.selectOptions(
    screen.getByRole("combobox", { name: "Runs" }),
    "custom"
  );
  const field = screen.getByRole("textbox", { name: "Cron expression 1" });
  await userEvent.clear(field);
  await userEvent.type(field, text);
  return field;
}

export { mocks };
