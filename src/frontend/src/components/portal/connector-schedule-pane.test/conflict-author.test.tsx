import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import type { ScheduleRead } from "@/api/connector-health-client";
import { ConnectorsApiError } from "@/api/connectors-client";
import { IdentityApiError, searchPersons } from "@/api/identity-client";
import type { PersonSearchResponse } from "@/api/identity-client";
import { AppTooltips } from "@/test/tooltip";
import { ConnectorScheduleEditor } from "../connector-schedule-editor";

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
vi.mock("@/api/identity-client", async (original) => ({
  ...(await original<typeof import("@/api/identity-client")>()),
  searchPersons: vi.fn(),
}));

const SELF = "00000000-0000-4000-8000-000000000001";
const OTHER = "00000000-0000-4000-8000-000000000002";
const NAMED = { items: [{ person_id: OTHER, display_name: "Example Admin" }] };
const SUFFIX = "It is reloaded below; check it and save again.";
let client: QueryClient;

function saved(personId: string, revision: number): ScheduleRead {
  return {
    etag: `"${revision}"`,
    schedule: {
      connector: "example-tracker",
      source_id: "main",
      revision,
      status: "applied",
      requested: {
        crons: [`0 ${revision + 6} * * 0`],
        updated_at: "2026-01-15T12:30:00Z",
        updated_by: personId,
      },
      live: null,
    },
  };
}

async function conflict(next = saved(OTHER, 2)) {
  const save = vi
    .fn()
    .mockRejectedValueOnce(new ConnectorsApiError(412, null))
    .mockResolvedValue(next);
  const reload = vi.fn().mockResolvedValue(next);
  const onClose = vi.fn();
  render(
    <QueryClientProvider client={client}>
      <AppTooltips>
        <ConnectorScheduleEditor
          read={saved(SELF, 1)}
          save={save}
          reload={reload}
          onClose={onClose}
        />
      </AppTooltips>
    </QueryClientProvider>
  );
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "0 9 * * 0" },
  });
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByRole("alert");
  return { save, reload, onClose };
}

beforeEach(() => {
  vi.resetAllMocks();
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});
afterEach(() => client.clear());

it("names the reloaded request's author once the lookup resolves", async () => {
  let finish!: (found: PersonSearchResponse) => void;
  vi.mocked(searchPersons).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    })
  );
  await conflict();
  expect(screen.getByRole("alert")).toHaveTextContent(
    `Another admin changed this schedule at 12:30 UTC. ${SUFFIX}`
  );
  expect(screen.getByRole("textbox")).toHaveValue("0 8 * * 0");
  await act(async () => finish(NAMED));
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent(
      `Example Admin changed this schedule at 12:30 UTC. ${SUFFIX}`
    )
  );
  expect(searchPersons).toHaveBeenCalledWith(
    OTHER,
    { limit: 1 },
    expect.any(AbortSignal)
  );
  expect(screen.getByRole("textbox")).toHaveValue("0 8 * * 0");
  expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
});

it("keeps edits and the reloaded revision when the author's name arrives", async () => {
  let finish!: (found: PersonSearchResponse) => void;
  vi.mocked(searchPersons).mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    })
  );
  const { save, reload, onClose } = await conflict();
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "0 10 * * 0" },
  });
  await act(async () => finish(NAMED));
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent("Example Admin changed")
  );
  expect(screen.getByRole("textbox")).toHaveValue("0 10 * * 0");
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Save" }));
  expect(save).toHaveBeenLastCalledWith('"2"', ["0 10 * * 0"]);
  expect(reload).toHaveBeenCalledTimes(1);
  expect(onClose).toHaveBeenCalledOnce();
});

it.each(["loading", "failed", "missing", "blank"])(
  "keeps Another admin when the author lookup is %s",
  async (state) => {
    if (state === "loading")
      vi.mocked(searchPersons).mockReturnValue(new Promise(() => {}));
    else if (state === "failed")
      vi.mocked(searchPersons).mockRejectedValue(
        new IdentityApiError(500, null)
      );
    else
      vi.mocked(searchPersons).mockResolvedValue({
        items:
          state === "missing" ? [] : [{ person_id: OTHER, display_name: " " }],
      });
    await conflict();
    await waitFor(() => expect(searchPersons).toHaveBeenCalledOnce());
    if (state !== "loading") {
      await waitFor(() =>
        expect(
          client.getQueryCache().find({
            queryKey: ["identity", "schedule-author", "example-scope", OTHER],
          })?.state.status
        ).toBe(state === "failed" ? "error" : "success")
      );
    }
    expect(screen.getByRole("alert")).toHaveTextContent(
      `Another admin changed this schedule at 12:30 UTC. ${SUFFIX}`
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent(OTHER);
    expect(screen.getByRole("textbox")).toHaveValue("0 8 * * 0");
  }
);

it("recognizes a save by the viewer in another tab", async () => {
  await conflict(saved(SELF, 2));
  expect(screen.getByRole("alert")).toHaveTextContent(
    `You changed this schedule in another tab at 12:30 UTC. ${SUFFIX}`
  );
  expect(searchPersons).not.toHaveBeenCalled();
});

it("keeps the existing conflict wording when the reloaded schedule has no request", async () => {
  const next = saved(OTHER, 2);
  next.schedule.requested = null;
  next.schedule.status = "inherited";
  next.schedule.live = {
    crons: ["0 3 * * *"],
    timezone: "UTC",
    origin: "default",
    suspended: false,
    observed_at: "2026-01-15T12:30:00Z",
  };
  await conflict(next);
  expect(screen.getByRole("alert")).toHaveTextContent(
    `Another admin changed this schedule at — UTC. ${SUFFIX}`
  );
  expect(searchPersons).not.toHaveBeenCalled();
});
