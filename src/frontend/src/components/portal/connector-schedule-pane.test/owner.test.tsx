import {
  QueryClient,
  QueryClientProvider,
  notifyManager,
} from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import * as api from "@/api/connector-health-client";
import { ConnectorsApiError } from "@/api/connectors-client";
import { searchPersons } from "@/api/identity-client";
import type {
  ConnectorHealth,
  ConnectorHealthSummary,
  ScheduleRead,
} from "@/api/connector-health-client";
import { ConnectorHealthPane } from "../connector-health";
import { AppTooltips } from "@/test/tooltip";
import { pickPreset } from "./preset";

vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: "example-session" }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: () => "example-scope",
}));
vi.mock("@/queries/identity-me", () => ({
  useMe: () => ({
    data: { person_id: "00000000-0000-4000-8000-000000000001" },
  }),
}));
vi.mock("@/api/connector-health-client", () => ({
  getConnectorHealth: vi.fn(),
  getConnectorSchedule: vi.fn(),
  getConnectorSyncs: vi.fn(),
  putConnectorSchedule: vi.fn(),
  deleteConnectorSchedule: vi.fn(),
}));
vi.mock("@/api/identity-client", () => ({ searchPersons: vi.fn() }));

const OTHER = "00000000-0000-4000-8000-000000000002";
const NOW = "2026-01-15T12:00:00Z";
let client: QueryClient;
let current: ScheduleRead;
let health: ConnectorHealthSummary;
const instance: ConnectorHealth = {
  connector: "example-tracker",
  source_id: "main",
  configured: true,
  schedule: {
    crons: ["0 6 * * *"],
    timezone: "UTC",
    origin: "api",
    suspended: false,
  },
};
function saved(revision: number, personId: string, cron: string): ScheduleRead {
  return {
    etag: `"${revision}"`,
    schedule: {
      connector: instance.connector,
      source_id: "main",
      revision,
      status: "applied",
      requested: { crons: [cron], updated_by: personId, updated_at: NOW },
      live: { ...instance.schedule!, crons: [cron], observed_at: NOW },
    },
  };
}
function anotherSave() {
  current = saved(2, OTHER, "0 9 * * 0");
  health = {
    ...health,
    connectors: [
      {
        ...instance,
        schedule_request: {
          revision: 2,
          crons: ["0 9 * * 0"],
          last_observation: { status: "applied", observed_at: NOW },
        },
      },
    ],
  };
}
function openPage() {
  return render(
    <QueryClientProvider client={client}>
      <ConnectorHealthPane />
    </QueryClientProvider>,
    { wrapper: AppTooltips }
  );
}
beforeEach(() => {
  vi.resetAllMocks();
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) =>
    window.setTimeout(() => callback(performance.now()), 16)
  );
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation((handle) =>
    window.clearTimeout(handle)
  );
  notifyManager.setScheduler(queueMicrotask);
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  current = saved(1, "00000000-0000-4000-8000-000000000001", "0 6 * * *");
  health = {
    as_of: NOW,
    checked_at: NOW,
    history_available: true,
    connectors: [instance],
  };
  vi.mocked(api.getConnectorHealth).mockImplementation(async () => health);
  vi.mocked(api.getConnectorSchedule).mockImplementation(async () => current);
  vi.mocked(api.getConnectorSyncs).mockResolvedValue({
    connector: instance.connector,
    syncs: [],
    window: 50,
  });
  vi.mocked(searchPersons).mockResolvedValue({
    items: [{ person_id: OTHER, display_name: "Example Admin" }],
  });
});
afterEach(() => {
  notifyManager.setScheduler((cb) => setTimeout(cb, 0));
  client.clear();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

it.each([true, false])(
  "a minute summary refresh updates another admin's schedule and author, expanded: %s",
  async (expanded) => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    openPage();
    const row = await screen.findByRole("row", { expanded: false });
    if (expanded) {
      await userEvent.click(row);
      await screen.findByText(/set by you/);
    }
    anotherSave();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    await waitFor(() =>
      expect(api.getConnectorHealth).toHaveBeenCalledTimes(2)
    );
    const scheduleButton = within(row).getByRole("button");
    await userEvent.hover(scheduleButton);
    expect(await screen.findByRole("tooltip")).toHaveTextContent("0 9 * * 0");
    await userEvent.unhover(scheduleButton);
    if (!expanded) await userEvent.click(row);
    await screen.findByText(/set by Example Admin/);
    expect(screen.getByText("0 9 * * 0")).toBeInTheDocument();
    expect(screen.queryByText(/overrides default/)).not.toBeInTheDocument();
    await userEvent.hover(
      screen.getByText(/set by Example Admin/).closest("[tabindex]")!
    );
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "Set by Example Admin"
    );
    expect(document.body.textContent).not.toContain(OTHER);
    expect(searchPersons).toHaveBeenCalledTimes(1);
  }
);

it("preserves a draft's revision through summary refresh and reloads the other schedule on 412", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  vi.mocked(api.putConnectorSchedule).mockRejectedValue(
    new ConnectorsApiError(412, null)
  );
  openPage();
  await userEvent.click(await screen.findByRole("row", { expanded: false }));
  await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
  await pickPreset("custom");
  fireEvent.change(screen.getByRole("textbox", { name: "Cron expression 1" }), {
    target: { value: "0 8 * * *" },
  });
  anotherSave();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000);
  });
  await waitFor(() =>
    expect(api.getConnectorSchedule).toHaveBeenCalledTimes(2)
  );
  expect(screen.getByRole("textbox")).toHaveValue("0 8 * * *");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(
    await screen.findByText(/Example Admin changed this schedule/)
  ).toBeInTheDocument();
  expect(api.putConnectorSchedule).toHaveBeenCalledWith(
    expect.objectContaining({
      connector: instance.connector,
      source_id: "main",
    }),
    '"1"',
    ["0 8 * * *"]
  );
  expect(screen.getByRole("textbox")).toHaveValue("0 9 * * 0");
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await screen.findByText(/set by Example Admin/);
  expect(document.body.textContent).not.toContain(OTHER);
});

it.each(["loading", "failed"])(
  "uses another admin in the pane and every pending tooltip when the lookup is %s",
  async (state) => {
    anotherSave();
    current.schedule.status = "pending";
    health.connectors[0].schedule_request!.last_observation!.status = "pending";
    if (state === "loading")
      vi.mocked(searchPersons).mockReturnValue(new Promise(() => {}));
    else
      vi.mocked(searchPersons).mockRejectedValue(
        new Error("Synthetic refusal")
      );
    openPage();
    const row = await screen.findByRole("row", { expanded: false });
    const chip = within(row).getByRole("img", {
      name: "Waiting for reconcile",
    });
    await waitFor(() => expect(api.getConnectorSchedule).toHaveBeenCalled());
    await userEvent.hover(chip);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "saved by another admin"
    );
    expect(screen.getByRole("tooltip")).not.toHaveTextContent(OTHER);
    await userEvent.unhover(chip);
    await userEvent.click(row);
    await screen.findByText(/set by another admin/);
    const header = screen
      .getByText("Waiting for reconcile")
      .closest("[tabindex]")!;
    await userEvent.hover(header);
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      "saved by another admin"
    );
    expect(document.body.textContent).not.toContain(OTHER);
  }
);

it.each([
  ["00000000-0000-4000-8000-000000000001", "you"],
  [OTHER, "Example Admin"],
])(
  "uses the same author for the pane and pending tooltips: %s",
  async (personId, name) => {
    anotherSave();
    current.schedule.requested!.updated_by = personId;
    current.schedule.status = "pending";
    health.connectors[0].schedule_request!.last_observation!.status = "pending";
    openPage();
    const row = await screen.findByRole("row", { expanded: false });
    await waitFor(() =>
      expect(
        client
          .getQueryCache()
          .findAll({ queryKey: ["connector-health", "schedule"] })[0].state
          .status
      ).toBe("success")
    );
    if (name !== "you")
      await waitFor(() =>
        expect(
          client
            .getQueryCache()
            .findAll({ queryKey: ["identity", "schedule-author"] })
            .some((q) => q.state.status === "success")
        ).toBe(true)
      );
    await userEvent.hover(
      within(row).getByRole("img", { name: "Waiting for reconcile" })
    );
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      `saved by ${name}`
    );
    await userEvent.unhover(
      within(row).getByRole("img", { name: "Waiting for reconcile" })
    );
    await userEvent.click(row);
    await screen.findByText(new RegExp(`set by ${name}`));
    await userEvent.hover(
      screen.getByText("Waiting for reconcile").closest("[tabindex]")!
    );
    expect(await screen.findByRole("tooltip")).toHaveTextContent(
      `saved by ${name}`
    );
    expect(document.body.textContent).not.toContain(personId);
  }
);
