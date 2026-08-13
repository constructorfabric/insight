// @vitest-environment jsdom
/**
 * The detail panel. What matters: a bound person known to the queue renders
 * as a recognisable card while an unknown one stays an honest bare id; the
 * history names the verb (and unknown reasons pass through — the vocabulary
 * is open); a stale shared link (404) says so instead of erroring; and an
 * unbound account is a stated fact, not an empty gap.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AccountBinding, AttentionItem } from "@/api/identity-client";
import { IdentityApiError } from "@/api/identity-client";

const binding = vi.hoisted(() => ({
  q: {
    data: undefined as AccountBinding | undefined,
    isLoading: false,
    isError: false,
    error: null as unknown,
    refetch: vi.fn(),
  },
}));
vi.mock("@/queries/identity-resolution", () => ({
  useAccountBinding: () => binding.q,
}));

import { AccountDetail } from "./account-detail";

const REF = {
  source: "github",
  source_id: "01900000-0000-7000-8000-00000000aa01",
  account_id: "dev-42",
};
const BOB = {
  person_id: "01900000-0000-7000-8000-0000000000b0",
  display_name: "Bob Park",
  email: "bob.park@example.com",
};

function queueItem(over: Partial<AttentionItem> = {}): AttentionItem {
  return {
    kind: "contested",
    ...REF,
    email: "dev42@example.com",
    username: null,
    candidates: [BOB],
    ...over,
  };
}

function bound(over: Partial<AccountBinding> = {}): AccountBinding {
  return { ...REF, person_id: BOB.person_id, history: [], ...over };
}

beforeEach(() => {
  binding.q.data = undefined;
  binding.q.isLoading = false;
  binding.q.isError = false;
  binding.q.error = null;
});

describe("AccountDetail", () => {
  it("shows the bound person as a card when the queue knows them", () => {
    binding.q.data = bound();
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText(/currently bound to/i)).toBeInTheDocument();
    expect(screen.getAllByText("Bob Park").length).toBeGreaterThan(0);
  });

  it("keeps an unknown bound person an honest bare id", () => {
    binding.q.data = bound({ person_id: "01900000-0000-7000-8000-00000000ffff" });
    render(<AccountDetail accountRef={REF} queueItem={queueItem({ candidates: [] })} />);

    expect(
      screen.getByText("01900000-0000-7000-8000-00000000ffff"),
    ).toBeInTheDocument();
  });

  it("states an unbound account as a fact", () => {
    binding.q.data = bound({ person_id: null });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText(/account is unresolved/i)).toBeInTheDocument();
  });

  it("names known verbs and passes unknown reasons through verbatim", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: "x",
          by_operator: true,
          reason: "operator-merge",
          recorded_at: "2026-08-01T10:15:00Z",
        },
        {
          person_id: BOB.person_id,
          author_person_id: "y",
          by_operator: false,
          reason: "seed-backfill",
          recorded_at: "2026-07-01T10:15:00Z",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText("Merged")).toBeInTheDocument();
    expect(screen.getByText("seed-backfill")).toBeInTheDocument();
    expect(screen.getByText(/by an operator/i)).toBeInTheDocument();
    expect(screen.getByText(/1 Aug 2026/)).toBeInTheDocument();
  });

  it("reads a 404 as a stale link, with no retry offered", () => {
    binding.q.isError = true;
    binding.q.error = new IdentityApiError(404, { title: "account not found" });
    render(<AccountDetail accountRef={REF} queueItem={undefined} />);

    expect(screen.getByText(/link may be stale/i)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /retry/i })).not.toBeInTheDocument();
  });

  it("offers a retry on any other failure", () => {
    binding.q.isError = true;
    binding.q.error = new IdentityApiError(500, {});
    render(<AccountDetail accountRef={REF} queueItem={undefined} />);

    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });
});
