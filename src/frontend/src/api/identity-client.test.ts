import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/api/fetch-with-auth", () => ({ fetchWithAuth: vi.fn() }));

import { fetchWithAuth } from "@/api/fetch-with-auth";

import {
  bindAccount,
  detachAccount,
  getAccountBinding,
  getAttention,
  getMe,
  getPerson,
  getPersonAccounts,
  IdentityApiError,
  mergePersons,
  searchPersons,
} from "./identity-client";

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

describe("getMe", () => {
  it("GETs /me and returns the caller with their roles verbatim", async () => {
    mockFetch.mockResolvedValueOnce(
      response({
        person_id: "019e27bc-dec0-7626-81a9-c5524662a6a9",
        insight_tenant_id: "t-1",
        roles: [
          { role_id: "a4d11000-0000-4000-8000-000000000001", name: "admin" },
        ],
      }),
    );

    const me = await getMe();

    expect(mockFetch.mock.calls[0][0]).toBe("/api/identity/v1/me");
    expect(me.roles).toEqual([
      { role_id: "a4d11000-0000-4000-8000-000000000001", name: "admin" },
    ]);
  });

  it("keeps an empty roles list as a valid 'not an admin' answer", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ person_id: "p-1", insight_tenant_id: "t-1", roles: [] }),
    );

    await expect(getMe()).resolves.toMatchObject({ roles: [] });
  });

  it("throws on an HTTP failure instead of pretending to an empty grant", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "unauthenticated" }, { ok: false, status: 401 }),
    );

    await expect(getMe()).rejects.toMatchObject({ status: 401 });
  });

  it("rejects a malformed body rather than reading it as 'no roles'", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ person_id: "p-1", insight_tenant_id: "t-1" } as never),
    );

    await expect(getMe()).rejects.toMatchObject({
      body: { error: "malformed_me" },
    });
  });

  // The gate reads `role_id` off every entry, so a well-shaped list of badly
  // shaped members has to fail here — downstream it would throw during render
  // instead of failing closed.
  it.each([
    ["a null member", [null]],
    ["a non-object member", ["admin"]],
    ["a member with no role_id", [{ name: "admin" }]],
    ["a member whose role_id is blank", [{ role_id: "  ", name: "admin" }]],
  ])("rejects a roles array with %s", async (_name, roles) => {
    mockFetch.mockResolvedValueOnce(
      response({ person_id: "p-1", insight_tenant_id: "t-1", roles } as never),
    );

    await expect(getMe()).rejects.toMatchObject({
      body: { error: "malformed_me" },
    });
  });
});

describe("getAttention", () => {
  it("GETs the review queue with its limit and returns items + rates", async () => {
    mockFetch.mockResolvedValueOnce(
      response({
        items: [
          {
            kind: "contested",
            source: "github",
            source_id: "01900000-0000-7000-8000-00000000aa01",
            account_id: "dev-42",
            email: "dev42@example.com",
            username: null,
            candidates: [],
          },
        ],
        rates: { observed: 1, bound: 0, pending: 1, no_evidence: 0, excluded: 0 },
      }),
    );

    const queue = await getAttention(50);

    expect(mockFetch.mock.calls[0][0]).toBe(
      "/api/identity/v1/resolution/attention?limit=50",
    );
    expect(queue.items).toHaveLength(1);
    expect(queue.rates.pending).toBe(1);
  });

  it("rejects a malformed body rather than reading it as an empty queue", async () => {
    mockFetch.mockResolvedValueOnce(response({ rates: null } as never));

    await expect(getAttention()).rejects.toMatchObject({
      body: { error: "malformed_attention" },
    });
  });

  it("throws on an HTTP failure (a revoked role must not look like zero work)", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "forbidden" }, { ok: false, status: 403 }),
    );

    await expect(getAttention()).rejects.toMatchObject({ status: 403 });
  });
});

describe("getAccountBinding", () => {
  it("URI-encodes every path segment — an account_id with slashes cannot escape", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ source: "github", source_id: "s", account_id: "a/b c", history: [] }),
    );

    await getAccountBinding({ source: "github", source_id: "s", account_id: "a/b c" });

    expect(mockFetch.mock.calls[0][0]).toBe(
      "/api/identity/v1/resolution/accounts/github/s/a%2Fb%20c",
    );
  });

  it("rejects a body without a history array", async () => {
    mockFetch.mockResolvedValueOnce(response({ source: "github" } as never));

    await expect(
      getAccountBinding({ source: "github", source_id: "s", account_id: "a" }),
    ).rejects.toMatchObject({ body: { error: "malformed_binding" } });
  });
});

describe("correction verbs", () => {
  const ACCOUNT = {
    source: "github",
    source_id: "01900000-0000-7000-8000-00000000aa01",
    id: "dev-42",
  };
  const OK = {
    applied: 1,
    already_decided: 0,
    items: [{ ...ACCOUNT, account_id: "dev-42", outcome: "applied" }],
  };

  it("bind wraps the single account into the bulk wire shape", async () => {
    mockFetch.mockResolvedValueOnce(response(OK));

    await bindAccount({ account: ACCOUNT, person_id: "p-1" });

    const [url, init] = mockFetch.mock.calls[0];
    expect(url).toBe("/api/identity/v1/resolution/bind");
    expect(JSON.parse((init as RequestInit).body as string)).toEqual({
      bindings: [{ account: ACCOUNT, person_id: "p-1" }],
      comment: "",
    });
  });

  it("merge names source and target explicitly", async () => {
    mockFetch.mockResolvedValueOnce(response(OK));

    await mergePersons({ source_person_id: "p-1", target_person_id: "p-2" });

    const [url, init] = mockFetch.mock.calls[0];
    expect(url).toBe("/api/identity/v1/resolution/merge");
    expect(JSON.parse((init as RequestInit).body as string)).toMatchObject({
      source_person_id: "p-1",
      target_person_id: "p-2",
    });
  });

  it("detach returns the freshly minted person", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ ...OK, new_person_id: "p-new" }),
    );

    const outcome = await detachAccount({ account: ACCOUNT });

    expect(outcome.new_person_id).toBe("p-new");
  });

  it("a refusal body without items is malformed, not silently empty", async () => {
    mockFetch.mockResolvedValueOnce(response({ applied: 0 } as never));

    await expect(bindAccount({ account: ACCOUNT, person_id: "p-1" })).rejects.toMatchObject({
      body: { error: "malformed_correction" },
    });
  });
});

describe("searchPersons", () => {
  it("URI-encodes the terms", async () => {
    mockFetch.mockResolvedValueOnce(response({ items: [], truncated: false }));

    await searchPersons("iva example.com");

    expect(mockFetch.mock.calls[0][0]).toBe(
      "/api/identity/v1/persons?q=iva%20example.com",
    );
  });

  it("rejects a malformed body rather than reading it as no matches", async () => {
    mockFetch.mockResolvedValueOnce(response({ truncated: false } as never));

    await expect(searchPersons("x y")).rejects.toMatchObject({
      body: { error: "malformed_search" },
    });
  });
});

describe("getPersonAccounts", () => {
  const OWNED = {
    person_id: "01900000-0000-7000-8000-0000000000b0",
    accounts: [
      {
        source: "github",
        source_id: "01900000-0000-7000-8000-00000000aa01",
        account_id: "gh-1",
        email: "gh-1@example.com",
        username: null,
        bound_by_operator: true,
      },
    ],
  };

  it("URI-encodes the person id and returns the accounts", async () => {
    mockFetch.mockResolvedValueOnce(response(OWNED));

    await expect(getPersonAccounts(OWNED.person_id)).resolves.toEqual(OWNED);
    expect(mockFetch.mock.calls[0][0]).toBe(
      `/api/identity/v1/resolution/persons/${OWNED.person_id}/accounts`,
    );
  });

  it("keeps an empty account list a valid answer", async () => {
    mockFetch.mockResolvedValueOnce(response({ person_id: "p-1", accounts: [] }));

    await expect(getPersonAccounts("p-1")).resolves.toMatchObject({ accounts: [] });
  });

  it("throws on an HTTP failure instead of reporting nothing to move", async () => {
    mockFetch.mockResolvedValueOnce(
      response({ title: "forbidden" }, { ok: false, status: 403 }),
    );

    await expect(getPersonAccounts("p-1")).rejects.toMatchObject({ status: 403 });
  });

  // The merge preview counts this list, so a malformed body must not read as
  // "no accounts move" — that would understate what the merge is about to do.
  it("rejects a malformed body rather than reading it as nothing to move", async () => {
    mockFetch.mockResolvedValueOnce(response({ person_id: "p-1" } as never));

    await expect(getPersonAccounts("p-1")).rejects.toMatchObject({
      body: { error: "malformed_accounts" },
    });
  });
});
