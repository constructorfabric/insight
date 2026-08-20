import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RouterHistory } from "@tanstack/react-router";

const mocks = vi.hoisted(() => ({ recordPageView: vi.fn() }));

vi.mock("@/telemetry", () => ({ recordPageView: mocks.recordPageView }));

import { GROUPS } from "@/lib/insight/groups";
import { MODES } from "@/lib/portal/identity-modes";
import { DIRECTIONS, ZONES, zoneItems } from "@/lib/portal/nav-model";

import { recordScreens } from "./usage-collection";

function historyStub(
  pathname: string,
  search = "",
): {
  history: RouterHistory;
  navigateRouterOnly: (pathname: string, search?: string) => void;
} {
  window.history.pushState({}, "", `${pathname}${search}`);
  const subscribers = new Set<() => void>();
  const location = { pathname, search };
  const history = {
    location,
    subscribe: (cb: () => void) => {
      subscribers.add(cb);
      return () => subscribers.delete(cb);
    },
  } as unknown as RouterHistory;
  return {
    history,
    navigateRouterOnly: (nextPathname: string, nextSearch = "") => {
      location.pathname = nextPathname;
      location.search = nextSearch;
      subscribers.forEach((cb) => cb());
    },
  };
}

describe("recordScreens", () => {
  beforeEach(() => {
    mocks.recordPageView.mockClear();
  });

  it("records the screen it starts on", () => {
    recordScreens(historyStub("/portal").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal");
  });

  it("composes the portal zone and item into the screen", () => {
    recordScreens(historyStub("/portal", "?zone=overview&item=trend").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/overview/trend");
  });

  it("records a screen once however often history fires", () => {
    const { history, navigateRouterOnly } = historyStub("/portal", "?zone=directions");
    recordScreens(history);
    navigateRouterOnly("/portal", "?zone=directions");

    expect(mocks.recordPageView).toHaveBeenCalledTimes(1);
  });

  it("records a zone change as the screen it opened, not the one it left", () => {
    const { history, navigateRouterOnly } = historyStub("/portal", "?zone=overview&item=trend");
    recordScreens(history);
    mocks.recordPageView.mockClear();

    navigateRouterOnly("/portal", "?zone=manage&item=identities");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/identities");
  });

  it("composes a person group item into the screen", () => {
    recordScreens(historyStub("/ic/:id/personal", "?item=git_output").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/ic/:id/personal/git_output");
  });

  it("drops a zone that names no screen in the rail", () => {
    recordScreens(historyStub("/portal", "?zone=user@example.com").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal");
  });

  it("drops an item that names no screen in the rail", () => {
    recordScreens(historyStub("/portal", "?zone=manage&item=user@example.com").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage");
  });
  it("composes an open direction and its lens into the screen", () => {
    recordScreens(historyStub("/portal", "?zone=directions&dir=dev&lens=Quality").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/directions/dev/quality");
  });

  it("records a lens change as its own screen", () => {
    const { history, navigateRouterOnly } = historyStub(
      "/portal",
      "?zone=directions&dir=dev&lens=Quality",
    );
    recordScreens(history);
    mocks.recordPageView.mockClear();

    navigateRouterOnly("/portal", "?zone=directions&dir=dev&lens=Delivery");

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/directions/dev/delivery");
  });

  it("slugs a lens whose name is not path-shaped", () => {
    recordScreens(
      historyStub("/portal", "?zone=directions&dir=collab&lens=Files+%26+sharing").history,
    );

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/directions/collab/files-sharing");
  });

  it("records the direction alone when no lens is open", () => {
    recordScreens(historyStub("/portal", "?zone=directions&dir=dev").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/directions/dev");
  });

  it("drops a lens the open direction does not have", () => {
    recordScreens(historyStub("/portal", "?zone=directions&dir=dev&lens=Meetings").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/directions/dev");
  });

  it("drops a direction that names none in the catalog", () => {
    recordScreens(
      historyStub("/portal", "?zone=directions&dir=user@example.com&lens=Quality").history,
    );

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/directions");
  });

  it("ignores a direction and lens another zone left in the URL", () => {
    recordScreens(
      historyStub("/portal", "?zone=manage&item=identities&dir=dev&lens=Quality").history,
    );

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/identities");
  });

  it("composes the identities console mode into the screen", () => {
    recordScreens(
      historyStub("/portal", "?zone=manage&item=identities&mode=person").history,
    );

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/identities/person");
  });

  it("records a retired mode as the screen it opens", () => {
    recordScreens(
      historyStub("/portal", "?zone=manage&item=identities&mode=people").history,
    );

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/identities/person");
  });

  it("records a mode the console does not offer as the one it falls back to", () => {
    recordScreens(
      historyStub("/portal", "?zone=manage&item=identities&mode=user@example.com").history,
    );

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/manage/identities/queue");
  });

  it("ignores a mode outside the identities console", () => {
    recordScreens(historyStub("/portal", "?zone=overview&item=trend&mode=person").history);

    expect(mocks.recordPageView).toHaveBeenCalledWith("/portal/overview/trend");
  });
});

function lensSlug(lens: string): string {
  return lens
    .toLowerCase()
    .split("")
    .map((c) => (/[a-z0-9]/.test(c) ? c : "-"))
    .join("")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

function everyScreen(): Array<{ search: string; pathname: string; screen: string }> {
  const states: Array<{ search: string; pathname: string; screen: string }> = [];

  for (const zone of ZONES) {
    const items = zoneItems(zone.id);
    if (!items.length) {
      states.push({
        pathname: "/portal",
        search: `?zone=${zone.id}`,
        screen: `/portal/${zone.id}`,
      });
      continue;
    }
    for (const item of items) {
      states.push({
        pathname: "/portal",
        search: `?zone=${zone.id}&item=${item.id}`,
        screen: `/portal/${zone.id}/${item.id}`,
      });
    }
  }

  for (const direction of DIRECTIONS) {
    states.push({
      pathname: "/portal",
      search: `?zone=directions&dir=${direction.id}`,
      screen: `/portal/directions/${direction.id}`,
    });
    for (const lens of direction.lenses) {
      states.push({
        pathname: "/portal",
        search: `?zone=directions&dir=${direction.id}&lens=${encodeURIComponent(lens)}`,
        screen: `/portal/directions/${direction.id}/${lensSlug(lens)}`,
      });
    }
  }

  for (const mode of MODES) {
    states.push({
      pathname: "/portal",
      search: `?zone=manage&item=identities&mode=${mode}`,
      screen: `/portal/manage/identities/${mode}`,
    });
  }

  for (const group of GROUPS) {
    states.push({
      pathname: "/ic/:id/personal",
      search: `?item=${group.id}`,
      screen: `/ic/:id/personal/${group.id}`,
    });
  }

  return states;
}

describe("every reachable screen", () => {
  beforeEach(() => {
    mocks.recordPageView.mockClear();
  });

  it.each(everyScreen())("records $search as $screen", ({ pathname, search, screen }) => {
    recordScreens(historyStub(pathname, search).history);

    expect(mocks.recordPageView).toHaveBeenCalledWith(screen);
  });

  it("gives every reachable state a screen of its own", () => {
    const states = everyScreen();
    const recorded = states.map(({ pathname, search }) => {
      mocks.recordPageView.mockClear();
      recordScreens(historyStub(pathname, search).history);
      return mocks.recordPageView.mock.calls[0]?.[0] as string;
    });

    expect(new Set(recorded).size).toBe(states.length);
  });
});
