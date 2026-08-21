// @vitest-environment jsdom
/**
 * The detail panel. What matters: a bound person known to the queue renders
 * as a recognisable card while an unknown one stays an honest bare id; the
 * history names the verb (and unknown reasons pass through — the vocabulary
 * is open); a stale shared link (200 with an empty journal, off the queue —
 * the read never 404s) says so and offers no verbs; and an unbound account
 * is a stated fact, not an empty gap.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";
import type { AccountBinding, AttentionItem } from "@/api/identity-client";

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

// The verbs have their own test file; here the panel's reads are under test.
vi.mock("@/components/portal/account-actions", () => ({
  AccountActions: ({ candidates }: { candidates: unknown[] }) => (
    <div data-testid="account-actions" data-candidates={candidates.length} />
  ),
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
const CAROL = {
  person_id: "01900000-0000-7000-8000-0000000000c0",
  display_name: "Carol Chen",
  email: "carol.chen@example.com",
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
  // The bound card and the verbs live in `AccountActions` now — see its suite.
  // What stays this window's job is handing the queue's candidates down.
  it("passes the queue's candidates to the decision surface", () => {
    binding.q.data = bound();
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByTestId("account-actions")).toHaveAttribute(
      "data-candidates",
      "1",
    );
  });

  it("names known verbs and passes unknown reasons through verbatim", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: "x",
          by_operator: true,
          reason: "operator-merge",
          // Zone-less on purpose — the real wire never carries a `Z`.
          recorded_at: "2026-08-01T10:15:00.000000",
        },
        {
          person_id: BOB.person_id,
          author_person_id: "y",
          by_operator: false,
          reason: "seed-backfill",
          recorded_at: "2026-07-01T10:15:00.000000",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText("Merged")).toBeInTheDocument();
    expect(screen.getByText("seed-backfill")).toBeInTheDocument();
    expect(screen.getByText(/by an operator/i)).toBeInTheDocument();
    // Two different questions: when exactly (comparable between entries,
    // pasteable into a ticket) and how long it has stood.
    expect(screen.getByText(/\d+ Aug 2026, \d\d:\d\d/)).toBeInTheDocument();
    expect(screen.getAllByText(/ago\)$/i).length).toBeGreaterThan(0);
    // The same person appears over and over in a trail; the id is what tells
    // two of them apart when the names do not.
    expect(screen.getAllByText(BOB.person_id).length).toBeGreaterThan(0);
  });

  // The resolver writes no reason at all — as an empty string, which is not
  // null, so a nullish fallback left the badge blank on every automatic entry.
  // Machine decision versus human decision is the one thing the badge carries.
  it("says a reasonless entry was automatic instead of rendering a blank badge", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: "00000000-0000-0000-0000-000000000000",
          by_operator: false,
          reason: "",
          recorded_at: "2026-08-01T10:15:00.000000",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText(/automatic/i)).toBeInTheDocument();
  });

  // First-login provisioning mints the binding during the sign-in itself; an
  // operator meeting a raw `login-bootstrap` learns nothing from it.
  it("names the first-sign-in provisioning reason", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: "00000000-0000-0000-0000-000000000000",
          by_operator: false,
          reason: "login-bootstrap",
          recorded_at: "2026-08-01T10:15:00.000000",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText(/first sign-in/i)).toBeInTheDocument();
    expect(screen.queryByText("login-bootstrap")).not.toBeInTheDocument();
  });

  // The batch mints this binding because the roster lists the account, not
  // because anything matched it. Same problem as above: a raw `roster-mint`
  // tells an operator nothing about what they are being asked to confirm.
  it("names the roster-mint reason", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: "00000000-0000-0000-0000-000000000000",
          by_operator: false,
          reason: "roster-mint",
          recorded_at: "2026-08-01T10:15:00.000000",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText(/added from the roster/i)).toBeInTheDocument();
    expect(screen.queryByText("roster-mint")).not.toBeInTheDocument();
  });

  // The comment is the one thing no other record holds — why a human did
  // this — and it was written to the operations journal from the first verb,
  // never read back. The reach matters beside it: a merge lands one row here
  // and one in every other account it moved.
  it("shows the operator call behind a decision: who, why and how far", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: CAROL.person_id,
          by_operator: true,
          reason: "operator-merge",
          recorded_at: "2026-08-01T10:15:00.000000",
        },
      ],
      operations: [
        {
          operation_id: "01900000-0000-7000-8000-0000000000f1",
          verb: "operator-merge",
          author_person_id: CAROL.person_id,
          author: CAROL,
          comment: "Same person — confirmed with HR.",
          accounts_touched: 12,
          outcome: "applied",
          recorded_at: "2026-08-01T10:15:00.000000",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText(/same person — confirmed with HR/i)).toBeInTheDocument();
    expect(screen.getByText(/12 accounts in this call/i)).toBeInTheDocument();
    expect(screen.getAllByText(/Carol Chen/).length).toBeGreaterThan(0);
    expect(screen.getByText("applied")).toBeInTheDocument();
  });

  // The service resolves the people a trail names, so an entry pointing at
  // somebody who is not a candidate stops being a bare id.
  it("names the person an entry points at, candidate or not", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: CAROL.person_id,
          person: CAROL,
          author_person_id: "00000000-0000-0000-0000-000000000000",
          by_operator: false,
          reason: "",
          recorded_at: "2026-07-01T10:15:00.000000",
        },
      ],
    });
    // CAROL is not in the queue item's candidates.
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    expect(screen.getByText("Carol Chen")).toBeInTheDocument();
  });

  // The voucher: a caller who can prove the account is observed (a search
  // hit, a person's own account list) gets the verbs even with an empty
  // journal — the stale-link guard exists for mistyped links, not for the
  // accounts an operator just found.
  it("offers the verbs on an empty journal when the caller vouches the account exists", () => {
    binding.q.data = bound({ person_id: null, history: [] });
    render(<AccountDetail accountRef={REF} queueItem={undefined} observed />);

    expect(screen.queryByText(/link may be stale/i)).not.toBeInTheDocument();
    expect(screen.getByTestId("account-actions")).toBeInTheDocument();
  });

  // The two records interleave by instant, newest first — an audit trail
  // that reads backwards, or shuffles a call away from its decision, tells a
  // false story. Ties keep the decision above its own call.
  it("interleaves decisions and calls newest-first", () => {
    binding.q.data = bound({
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: CAROL.person_id,
          by_operator: true,
          reason: "operator-bind",
          recorded_at: "2026-08-10T10:00:00.000000",
        },
        {
          person_id: BOB.person_id,
          author_person_id: "00000000-0000-0000-0000-000000000000",
          by_operator: false,
          reason: "",
          recorded_at: "2026-07-01T10:00:00.000000",
        },
      ],
      operations: [
        {
          operation_id: "01900000-0000-7000-8000-0000000000f1",
          verb: "operator-detach",
          author_person_id: CAROL.person_id,
          author: CAROL,
          comment: null,
          accounts_touched: 1,
          outcome: "applied",
          recorded_at: "2026-07-20T10:00:00.000000",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={queueItem()} />);

    const rows = screen.getAllByRole("listitem");
    const texts = rows.map((row) => row.textContent ?? "");
    expect(texts[0]).toMatch(/Bound/);
    expect(texts[1]).toMatch(/Detached/);
    expect(texts[2]).toMatch(/Automatic/);
  });

  it("reads an off-queue empty journal as a stale link, offering no verbs", () => {
    binding.q.data = bound({ person_id: null, history: [] });
    render(<AccountDetail accountRef={REF} queueItem={undefined} />);

    expect(screen.getByText(/link may be stale/i)).toBeInTheDocument();
    expect(screen.queryByTestId("account-actions")).not.toBeInTheDocument();
  });

  it("keeps an off-queue account with history actionable — decided is not stale", () => {
    binding.q.data = bound({
      person_id: null,
      history: [
        {
          person_id: BOB.person_id,
          author_person_id: "x",
          by_operator: true,
          reason: "operator-exclude",
          recorded_at: "2026-08-01T10:15:00.000000",
        },
      ],
    });
    render(<AccountDetail accountRef={REF} queueItem={undefined} />);

    expect(screen.queryByText(/link may be stale/i)).not.toBeInTheDocument();
    expect(screen.getByTestId("account-actions")).toBeInTheDocument();
  });

  it("offers a retry on a failed read", () => {
    binding.q.isError = true;
    binding.q.error = new Error("identity is down");
    render(<AccountDetail accountRef={REF} queueItem={undefined} />);

    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });
});
