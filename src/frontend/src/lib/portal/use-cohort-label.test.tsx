// @vitest-environment jsdom
/**
 * The noun a peer comparison uses. Three surfaces hardcoded it while injecting
 * stats from the slice cohort, so "vs department median" could sit over
 * manager-cohort numbers — a comparison naming the wrong pool of people, which
 * is worse than one naming none.
 */
vi.mock("@tanstack/react-router", async () => {
  const { portalRouterMock } = await import("@/test/portal-router");
  return portalRouterMock();
});

import { renderHook } from "@testing-library/react";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { identityPerson, pid } from "@/test/identity";
import type { IdentityPerson } from "@/types/insight";

const mocks = vi.hoisted(() => ({ tree: undefined as IdentityPerson | undefined }));

vi.mock("@/auth", () => ({
  useViewer: () => ({ email: "boss@x", personId: pid("boss") }),
}));
vi.mock("@/queries/ic-dashboard", () => ({
  useIcPerson: () => ({ data: mocks.tree }),
}));

import { portalRouter } from "@/test/portal-router";

import { useCohortLabel } from "./use-cohort-label";

const person = (
  label: string,
  attrs: Partial<IdentityPerson> = {},
  subs: IdentityPerson[] = [],
): IdentityPerson => identityPerson(label, attrs, subs);

beforeEach(() => {
  portalRouter.reset();
  // A roster that offers two real dimensions, so a slice has something to name.
  mocks.tree = person("boss", {}, [
    person("a", { division: "R&D", job_title: "Engineer" }),
    person("b", { division: "Sales", job_title: "Rep" }),
    person("c", { division: "R&D", job_title: "Engineer" }),
  ]);
});

describe("useCohortLabel", () => {
  it("says 'team' when the roster is one undivided cohort", () => {
    expect(renderHook(() => useCohortLabel()).result.current).toBe("team");
  });

  it("names the active slice's own dimension", () => {
    act(() => portalRouter.set({ slice: "division" }));
    expect(renderHook(() => useCohortLabel()).result.current).toBe("division");
  });

  it("falls back to 'cohort' for a slice the roster cannot offer", () => {
    // A shared link may name a dimension this org does not have. Saying
    // "cohort" is honest; echoing the unknown key would invent a group.
    act(() => portalRouter.set({ slice: "office" }));
    expect(renderHook(() => useCohortLabel()).result.current).toBe("cohort");
  });

  it("never returns a capitalised label — it reads mid-sentence", () => {
    act(() => portalRouter.set({ slice: "division" }));
    const label = renderHook(() => useCohortLabel()).result.current;
    expect(label).toBe(label.toLowerCase());
  });
});
