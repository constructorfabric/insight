import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import { getPerson, IdentityApiError } from "./identity-client";

const mockFetch = fetchWithAuth as unknown as ReturnType<typeof vi.fn>;

function response(body: unknown, init?: { ok?: boolean; status?: number }): Response {
  return {
    ok: init?.ok ?? true,
    status: init?.status ?? 200,
    json: async () => body,
  } as unknown as Response;
}

beforeEach(() => {
  mockFetch.mockReset();
});

describe("getPerson", () => {
  it("POSTs /profiles with a person_id body and maps the profile", async () => {
    mockFetch.mockResolvedValueOnce(
      response({
        person_id: "019e27bc-dec0-7626-81a9-c5524662a6a9",
        insight_tenant_id: "t-1",
        email: "bob.park@example.com",
        display_name: "Bob Park",
        job_title: "Lead",
        supervisor_email: "ceo@example.com",
      }),
    );

    const person = await getPerson("019e27bc-dec0-7626-81a9-c5524662a6a9");

    const [url, init] = mockFetch.mock.calls[0];
    expect(url).toBe("/api/identity/v1/profiles");
    expect(init).toMatchObject({ method: "POST" });
    expect(JSON.parse((init as RequestInit).body as string)).toEqual({
      value_type: "person_id",
      value: "019e27bc-dec0-7626-81a9-c5524662a6a9",
    });

    expect(person.person_id).toBe("019e27bc-dec0-7626-81a9-c5524662a6a9");
    expect(person.email).toBe("bob.park@example.com");
    expect(person.job_title).toBe("Lead");
    expect(person.supervisor_email).toBe("ceo@example.com");
    // Omitted optional strings default to ""; omitted parent fields stay null.
    expect(person.department).toBe("");
    expect(person.parent_id).toBeNull();
    expect(person.parent_email).toBeNull();
  });

  it("keeps subordinates without an email — person_id is the key now", async () => {
    mockFetch.mockResolvedValueOnce(
      response({
        person_id: "019e27bc-dec0-7626-81a9-c5524662a6aa",
        insight_tenant_id: "t-1",
        email: "lead@example.com",
        subordinates: [
          { person_id: "019e27bc-dec0-7626-81a9-c5524662a6ab", insight_tenant_id: "t-1", email: "ic1@example.com" },
          // No email is legitimate: identity serves persons whose log carries
          // none, and links/keys read person_id.
          { person_id: "019e27bc-dec0-7626-81a9-c5524662a6a9", insight_tenant_id: "t-1" },
        ],
      }),
    );

    const person = await getPerson("019e27bc-dec0-7626-81a9-c5524662a6aa");

    expect(person.subordinates.map((s) => s.person_id)).toEqual([
      "019e27bc-dec0-7626-81a9-c5524662a6ab",
      "019e27bc-dec0-7626-81a9-c5524662a6a9",
    ]);
    expect(person.subordinates.map((s) => s.email)).toEqual([
      "ic1@example.com",
      "",
    ]);
  });

  it("drops a subordinate without a person_id — a keyless node breaks links and React keys", async () => {
    mockFetch.mockResolvedValueOnce(
      response({
        person_id: "019e27bc-dec0-7626-81a9-c5524662a6aa",
        insight_tenant_id: "t-1",
        subordinates: [
          { person_id: "019e27bc-dec0-7626-81a9-c5524662a6ab", insight_tenant_id: "t-1" },
          { insight_tenant_id: "t-1", email: "keyless@example.com" },
          { person_id: "  ", insight_tenant_id: "t-1" },
        ],
      } as never),
    );

    const person = await getPerson("019e27bc-dec0-7626-81a9-c5524662a6aa");

    expect(person.subordinates.map((s) => s.person_id)).toEqual(["019e27bc-dec0-7626-81a9-c5524662a6ab"]);
  });

  it("throws IdentityApiError with the status + body on a non-ok response", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ error: "not_found" }, { ok: false, status: 404 }),
    );

    await expect(getPerson("ghost@example.com")).rejects.toMatchObject({
      name: "IdentityApiError",
      status: 404,
      body: { error: "not_found" },
    });
  });

  it("throws IdentityApiError(invalid_json) when the body is not JSON", async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      status: 200,
      json: async () => {
        throw new SyntaxError("Unexpected token");
      },
    } as unknown as Response);

    await expect(getPerson("bob@example.com")).rejects.toMatchObject({
      status: 200,
      body: { error: "invalid_json" },
    });
  });

  it("rejects a profile missing the required person_id", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ insight_tenant_id: "t-1", email: "bob@example.com" } as never),
    );

    const err = await getPerson("019e27bc-dec0-7626-81a9-c5524662a6a9").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(IdentityApiError);
    expect((err as IdentityApiError).body).toEqual({
      error: "missing_person_id",
    });
  });

});
