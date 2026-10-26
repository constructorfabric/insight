import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/identity-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/api/identity-client")>()),
  getPerson: vi.fn(),
  searchPersons: vi.fn(),
}));

import * as identityClient from "@/api/identity-client";

import { personNameQuery } from "./person-name";

const PERSON = "01900000-0000-7000-8000-00000000000a";

const read = (source: "tenant" | "roster") =>
  personNameQuery(PERSON, source).queryFn?.({ signal: undefined } as never);

beforeEach(() => {
  vi.resetAllMocks();
});

describe("personNameQuery", () => {
  it("names a person through the tenant search for an administrator", async () => {
    vi.mocked(identityClient.searchPersons).mockResolvedValue({
      items: [{ person_id: PERSON, display_name: "Jane Doe" } as never],
    });

    await expect(read("tenant")).resolves.toBe("Jane Doe");
    expect(identityClient.getPerson).not.toHaveBeenCalled();
  });

  it("names a person through the roster for anyone else", async () => {
    vi.mocked(identityClient.getPerson).mockResolvedValue({
      person_id: PERSON,
      display_name: "Jane Doe",
    } as Awaited<ReturnType<typeof identityClient.getPerson>>);

    await expect(read("roster")).resolves.toBe("Jane Doe");
    expect(identityClient.getPerson).toHaveBeenCalledWith(PERSON, undefined);
    expect(identityClient.searchPersons).not.toHaveBeenCalled();
  });

  it.each([
    ["a person with no name", [{ person_id: PERSON, display_name: null }]],
    ["a person the search does not find", []],
  ])(
    "falls back to the id through the tenant search for %s",
    async (_case, items) => {
      vi.mocked(identityClient.searchPersons).mockResolvedValue({
        items: items as never,
      });

      await expect(read("tenant"), `case: ${_case}`).resolves.toBe(PERSON);
    }
  );

  it("never answers the id through the roster for a person with no name", async () => {
    vi.mocked(identityClient.getPerson).mockResolvedValue({
      person_id: PERSON,
      display_name: null,
    } as never);

    await expect(read("roster")).resolves.toBeNull();
  });

  it("fails when the roster refuses the person, so no owner is shown", async () => {
    vi.mocked(identityClient.getPerson).mockRejectedValue(
      new identityClient.IdentityApiError(404, null)
    );

    await expect(read("roster")).rejects.toBeInstanceOf(
      identityClient.IdentityApiError
    );
  });

  it("keeps the two sources under separate keys", () => {
    expect(personNameQuery(PERSON, "tenant").queryKey).not.toEqual(
      personNameQuery(PERSON, "roster").queryKey
    );
  });
});
