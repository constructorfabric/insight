/**
 * The two things the collector must NOT do, neither of which the report can
 * catch after the fact.
 *
 * `telemetry.test.ts` covers what it sends. This module covers the refusals —
 * both of which are a single expression in `startUsageTelemetry`, and neither
 * of which leaves a trace anywhere a later assertion could find: a view-as
 * session that recorded is indistinguishable in the store from an ordinary
 * one, and captured on-screen text is captured.
 *
 * Its own file because both need `createTelemetry` itself under assertion —
 * whether it was called, and with what — where `telemetry.test.ts` mocks it to
 * a bare factory that discards its options.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Session } from "@/auth/types";

const mocks = vi.hoisted(() => ({ createTelemetry: vi.fn(), logEvent: vi.fn() }));

vi.mock("@gears-frontx/telemetry", () => ({ createTelemetry: mocks.createTelemetry }));

vi.mock("@/api/usage-client", () => ({
  getUsageConfig: () => Promise.resolve({ enabled: true }),
}));

function fakeService() {
  const service = {
    identify: () => service,
    start: () => service,
    logEvent: mocks.logEvent,
    destroy: () => {},
  };
  return service;
}

/** The service handle is module state, so each test needs its own module. */
async function freshTelemetry() {
  vi.resetModules();
  return import("./telemetry");
}

function session(impersonatorEmail: string | null): Session {
  return { personId: "cccccccc-0000-0000-0000-000000000001", impersonatorEmail } as Session;
}

describe("usage collection refuses", () => {
  beforeEach(() => {
    mocks.createTelemetry.mockReset();
    mocks.createTelemetry.mockImplementation(() => fakeService());
    mocks.logEvent.mockReset();
  });

  it("records nothing in a session opened on somebody else's behalf", async () => {
    // An operator viewing the product as somebody else is not that person
    // using it, and is not the operator using it either. Counting it would put
    // visits on a person who was not there — and the store cannot tell the two
    // apart afterwards, so nothing downstream can undo it.
    const telemetry = await freshTelemetry();

    await telemetry.startUsageTelemetry(session("operator@company.example"));

    expect(mocks.createTelemetry).not.toHaveBeenCalled();

    telemetry.recordPageView("/portal/manage/identities");
    telemetry.recordUsageEvent("drill", "git.commits");

    expect(mocks.logEvent).not.toHaveBeenCalled();
  });

  it("starts for an ordinary session, so the refusal above is not vacuous", async () => {
    const telemetry = await freshTelemetry();

    await telemetry.startUsageTelemetry(session(null));

    expect(mocks.createTelemetry).toHaveBeenCalledTimes(1);
    telemetry.recordPageView("/portal/manage/identities");
    expect(mocks.logEvent).toHaveBeenCalledWith("page_view", {
      path: "/portal/manage/identities",
    });
  });

  it("never lets the SDK capture what is written on the screen", async () => {
    // `autocapture` defaults to ON in the library, and what it captures is the
    // text of whatever was clicked — names, filter values, whatever a person
    // typed. One deleted word turns it back on, and the events it produces
    // look like ordinary usage once stored.
    const telemetry = await freshTelemetry();

    await telemetry.startUsageTelemetry(session(null));

    const options = mocks.createTelemetry.mock.calls[0]?.[0] as { autocapture?: unknown };
    expect(options.autocapture).toBe(false);
  });

  it("sends the screen and the action label, and nothing else", async () => {
    const telemetry = await freshTelemetry();
    await telemetry.startUsageTelemetry(session(null));

    telemetry.recordPageView("/ic/cccccccc-0000-0000-0000-000000000001/personal");
    telemetry.recordUsageEvent("period", "month");

    const sent = mocks.logEvent.mock.calls as Array<[string, Record<string, unknown>]>;
    expect(sent.length).toBe(2);
    for (const [, data] of sent) {
      expect(Object.keys(data).sort().join(",")).toMatch(/^(path|path,target)$/);
    }
    // The person the screen was about is reduced before it leaves the browser.
    expect(sent[0][1].path).toBe("/ic/:id/personal");
  });
});
