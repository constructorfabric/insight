import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import * as identityClient from "@/api/identity-client";
import { wrapper } from "@/test/alerts";

vi.mock("@/api/identity-client");

import { useSupervisorName } from "./supervisor-name";

const getSupervisor = vi.mocked(identityClient.getSupervisor);

beforeEach(() => {
  vi.clearAllMocks();
});

describe("useSupervisorName", () => {
  it("names the person's supervisor", async () => {
    getSupervisor.mockResolvedValue({ display_name: "Chief Example" });

    const { result } = renderHook(() => useSupervisorName("boss"), { wrapper });

    await waitFor(() => expect(result.current).toBe("Chief Example"));
    expect(getSupervisor).toHaveBeenCalledWith("boss", expect.anything());
  });

  it("asks for no one without a person", () => {
    const { result } = renderHook(() => useSupervisorName(null), { wrapper });

    expect(result.current).toBeNull();
    expect(getSupervisor).not.toHaveBeenCalled();
  });
});
