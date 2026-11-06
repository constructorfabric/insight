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
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import {
  afterAll,
  afterEach,
  beforeAll,
  beforeEach,
  expect,
  it,
  vi,
} from "vitest";

import * as scheduleApi from "@/api/connector-health-client";

import type {
  ConnectorHealth,
  ScheduleRead,
} from "@/api/connector-health-client";

vi.mock("@/auth/use-auth", () => ({
  useAuth: () => ({ session: "synthetic-session" }),
}));
vi.mock("@/auth/session-scope", () => ({
  sessionAuthorizationScope: () => "synthetic-scope",
}));
vi.mock("@/queries/identity-me", () => ({
  useMe: () => ({ data: { person_id: "example-admin" } }),
}));

import { ConnectorSchedulePane } from "../connector-schedule-pane";
import { AppTooltips } from "@/test/tooltip";

const row: ConnectorHealth = {
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
const path = "*/api/analytics/v1/connectors/example-tracker/main/schedule";
const server = setupServer();
let client: QueryClient;
let current: ScheduleRead;
let puts: { etag: string | null; body: unknown }[];

function gate<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function changed(
  revision: number,
  cron: string,
  status: "pending" | "applied" = "pending"
): ScheduleRead {
  return {
    etag: `"${revision}"`,
    schedule: {
      connector: row.connector,
      source_id: "main",
      revision,
      requested: {
        crons: [cron],
        updated_by: "example-admin",
        updated_at: "2026-01-15T12:00:00Z",
      },
      applied: {
        ...row.schedule!,
        crons: status === "applied" ? [cron] : row.schedule!.crons,
        origin: status === "applied" ? "api" : "default",
        revision: status === "applied" ? revision : 0,
        observed_at: "2026-01-15T12:10:00Z",
      },
      status,
    },
  };
}
function reply(read: ScheduleRead) {
  return HttpResponse.json(read.schedule, { headers: { ETag: read.etag } });
}
async function open() {
  render(
    <QueryClientProvider client={client}>
      <ConnectorSchedulePane row={row} polling={false} />
    </QueryClientProvider>,
    { wrapper: AppTooltips }
  );
  await userEvent.click(await screen.findByRole("button", { name: "Edit" }));
  await userEvent.selectOptions(
    screen.getByRole("combobox", { name: "Runs" }),
    "custom"
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Cron expression 1" }), {
    target: { value: "0 8 * * *" },
  });
}
async function refresh() {
  await act(async () => {
    await client.invalidateQueries({
      queryKey: ["connector-health", "schedule"],
    });
  });
}

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterAll(() => server.close());
beforeEach(() => {
  notifyManager.setScheduler(queueMicrotask);
  current = changed(1, "0 6 * * *");
  puts = [];
  client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false },
    },
  });
  server.use(
    http.get(path, () => reply(current)),
    http.get("*/api/analytics/v1/connector-health/example-tracker/syncs", () =>
      HttpResponse.json({ connector: row.connector, syncs: [], window: 7 })
    ),
    http.put(path, async ({ request }) => {
      const etag = request.headers.get("If-Match");
      const body = (await request.json()) as { crons: string[] };
      puts.push({ etag, body });
      if (etag !== current.etag)
        return HttpResponse.json(
          { status: 412 },
          { status: 412, headers: { ETag: current.etag } }
        );
      current = changed(current.schedule.revision + 1, body.crons[0]);
      return reply(current);
    })
  );
});
afterEach(() => {
  notifyManager.setScheduler((callback) => setTimeout(callback, 0));
  client.clear();
  vi.restoreAllMocks();
  server.resetHandlers();
  vi.useRealTimers();
});

it("a background poll preserves the draft and Save reloads a pending conflict", async () => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  await open();
  const entered = gate<void>();
  const release = gate<void>();
  current = changed(2, "0 9 * * 0");
  let first = true;
  server.use(
    http.get(path, async () => {
      if (first) {
        first = false;
        entered.resolve();
        await release.promise;
      }
      return reply(current);
    })
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(30_000);
  });
  await entered.promise;
  await act(async () => {
    release.resolve();
  });
  await waitFor(() =>
    expect(
      client.getQueryData<ScheduleRead>([
        "connector-health",
        "schedule",
        "example-tracker//main",
        "synthetic-scope",
      ])?.schedule.revision
    ).toBe(2)
  );
  expect(screen.getByRole("textbox")).toHaveValue("0 8 * * *");
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(
    await screen.findByText(/Another admin changed this schedule/)
  ).toBeInTheDocument();
  expect(puts).toEqual([{ etag: '"1"', body: { crons: ["0 8 * * *"] } }]);
  expect(screen.getByRole("textbox")).toHaveValue("0 9 * * 0");
  expect(screen.getByText("● Pending")).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(
    screen.getByText(/Saved. Until the request is applied/)
  ).toBeInTheDocument();
});

it("a double click on Save sends one PUT while its response is held", async () => {
  const entered = gate<void>();
  const release = gate<void>();
  server.use(
    http.put(path, async ({ request }) => {
      puts.push({
        etag: request.headers.get("If-Match"),
        body: await request.json(),
      });
      entered.resolve();
      await release.promise;
      current = changed(2, "0 8 * * *");
      return reply(current);
    })
  );
  await open();
  await userEvent.dblClick(screen.getByRole("button", { name: "Save" }));
  await entered.promise;
  expect(puts).toHaveLength(1);
  await act(async () => {
    release.resolve();
  });
  await screen.findByRole("button", { name: "Edit" });
  expect(puts).toHaveLength(1);
});

it("a stale GET resolving after a successful PUT cannot restore the old revision", async () => {
  await open();
  const entered = gate<void>();
  const release = gate<void>();
  const stale = current;
  const arrived = gate<void>();
  const getSchedule = scheduleApi.getConnectorSchedule;
  vi.spyOn(scheduleApi, "getConnectorSchedule").mockImplementation(
    async (instance) => {
      const result = await getSchedule(instance);
      if (result.etag === stale.etag) arrived.resolve();
      return result;
    }
  );
  let first = true;
  server.use(
    http.get(path, async () => {
      if (first) {
        first = false;
        entered.resolve();
        await release.promise;
        return reply(stale);
      }
      return reply(current);
    })
  );
  const background = client.invalidateQueries({
    queryKey: ["connector-health", "schedule"],
  });
  await entered.promise;
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByRole("button", { name: "Edit" });
  await act(async () => {
    release.resolve();
    await arrived.promise;
    await background;
  });
  await waitFor(() =>
    expect(
      client.getQueryData<ScheduleRead>([
        "connector-health",
        "schedule",
        "example-tracker//main",
        "synthetic-scope",
      ])?.schedule.revision
    ).toBe(2)
  );
  expect(screen.getByText(/0 8 \* \* \*/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Edit" }));
  await userEvent.selectOptions(screen.getByRole("combobox"), "custom");
  expect(screen.getByRole("textbox")).toHaveValue("0 8 * * *");
});

it("an applied poll keeps the editor open and Cancel reveals applied status", async () => {
  await open();
  current = changed(1, "0 6 * * *", "applied");
  await refresh();
  expect(screen.getByRole("textbox")).toHaveValue("0 8 * * *");
  expect(screen.getByRole("button", { name: "Save" })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.getByText("Applied")).toBeInTheDocument();
  expect(screen.getByText(/Applied at/)).toBeInTheDocument();
  expect(
    screen.queryByText(/Saved. Until the request is applied/)
  ).not.toBeInTheDocument();
});
