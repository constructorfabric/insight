import { fetchWithAuth } from "@/api/fetch-with-auth";
import { normalizePersonId } from "@/lib/metrics/entity";
import type { IdentityPerson } from "@/types/insight";

const BASE =
  (import.meta.env.VITE_IDENTITY_BASE as string | undefined) ??
  "/api/identity/v1";

/** One active role assignment of the caller, as `GET /me` reports it. */
export interface MeRole {
  role_id: string;
  name: string;
}

/**
 * Wire shape of `GET /me` — who the gateway JWT identifies, with their active
 * identity roles. `roles` comes from the identity service's `person_roles`
 * table (the same rows every admin endpoint's gate checks) — NOT from the
 * login token's realm roles, which no identity endpoint reads. An empty list
 * IS the "not an admin" answer; the endpoint never 403s.
 */
/**
 * Whose data a caller may see, as the identity service decides it: the
 * reporting line plus explicit grants, or every person in the tenant.
 */
export type VisibilityPolicy = "org_chart" | "flat";

export interface MeResponse {
  person_id: string;
  insight_tenant_id: string;
  roles: MeRole[];
  /** Absent from an older service; readers treat that as `org_chart`. */
  visibility_policy?: VisibilityPolicy;
}

/**
 * The caller's identity and active roles. Live on every call: granting or
 * revoking a role is visible on the next fetch, no re-login needed.
 */
export async function getMe(): Promise<MeResponse> {
  const res = await fetchWithAuth(`${BASE}/me`);
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let me: MeResponse;
  try {
    me = (await res.json()) as MeResponse;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  // A malformed answer must read as "roles unknown", never as "no roles" —
  // the admin gate downstream fails closed either way, but an error is
  // diagnosable where a silent [] is not. The entries are checked too: the
  // gate reads `role_id` off each one, so a `[null]` would throw during
  // render rather than fail closed.
  if (
    !me.person_id?.trim() ||
    !Array.isArray(me.roles) ||
    !me.roles.every(isMeRole)
  ) {
    throw new IdentityApiError(res.status, { error: "malformed_me" });
  }
  return me;
}

function isMeRole(role: unknown): role is MeRole {
  return (
    typeof role === "object" &&
    role !== null &&
    typeof (role as MeRole).role_id === "string" &&
    (role as MeRole).role_id.trim() !== ""
  );
}

/** A person as operator surfaces display them — the wire `PersonSummaryResponse`. */
export interface PersonSummary {
  person_id: string;
  /** The journal holds nothing but an automatic mint for them — a sign-in that
   *  needed somebody to enter as, or a roster listing an account with no
   *  address. They may duplicate a person the roster knows, so never a merge
   *  target: the history is on the other side. */
  provisional?: boolean;
  email?: string | null;
  username?: string | null;
  display_name?: string | null;
  job_title?: string | null;
  status?: string | null;
}

/** One account awaiting an operator decision. */
export interface AttentionItem {
  /** `contested` | `binding_conflict` | `provisioned_at_login` |
   *  `minted_from_roster` | `no_source_id` | `no_evidence` — open vocabulary. */
  kind: string;
  source: string;
  source_id: string;
  account_id: string;
  email?: string | null;
  username?: string | null;
  /** How the source describes the account. None of it is matchable — it is
   *  what lets an operator recognise an account automation cannot place. */
  display_name?: string | null;
  job_title?: string | null;
  department?: string | null;
  status?: string | null;
  manager_email?: string | null;
  /** Who holds the account right now; absent = nobody. Names which of the
   *  candidates is the one being disagreed with. */
  bound_to?: string | null;
  /** Hydrated person cards, not bare ids. */
  candidates: PersonSummary[];
}

/** Counts over EVERY observed account, regardless of the item cap. */
export interface ResolutionRates {
  observed: number;
  bound: number;
  pending: number;
  /** Unbound accounts no seed run can bind — an operator decides, or nobody. */
  no_source_id: number;
  no_evidence: number;
  excluded: number;
}

export interface AttentionResponse {
  items: AttentionItem[];
  rates: ResolutionRates;
  /** One of the reads behind the queue hit its safety cap: the queue and the
   *  rates describe only part of the tenant's accounts. Optional so a client
   *  deployed ahead of the backend keeps working; absent reads as complete. */
  truncated?: boolean;
  /** `limit` cut the item list — the rates are still whole-tenant, only this
   *  page is short. Optional for the same reason as {@link truncated}. */
  items_truncated?: boolean;
}

/** The queue's first read — a healthy backlog arrives whole. */
export const QUEUE_FIRST_PAGE = 200;

/** The service's own ceiling on `limit`: asking past it answers the same page,
 *  so a reader who has reached it is told to work the backlog down instead.
 *
 *  INVARIANT: mirrors `MAX_QUEUE_LIMIT` in the identity-resolution service. */
export const QUEUE_MAX_ITEMS = 1000;

/**
 * The operator review queue (`GET /resolution/attention`) — accounts the
 * resolver could not decide, with the tenant-wide match rate. Admin-gated
 * server-side; the caller is expected to sit behind `useIsAdmin`.
 *
 * There is no cursor: the queue is derived from the whole tenant on every
 * read, so asking for more of it is a larger `limit`, not a next page.
 */
export async function getAttention(
  limit: number = QUEUE_FIRST_PAGE
): Promise<AttentionResponse> {
  const res = await fetchWithAuth(
    `${BASE}/resolution/attention?limit=${limit}`
  );
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let attention: AttentionResponse;
  try {
    attention = (await res.json()) as AttentionResponse;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(attention.items) || attention.rates == null) {
    throw new IdentityApiError(res.status, { error: "malformed_attention" });
  }
  return attention;
}

/** One account a search matched, with whose it is. */
export interface AccountMatch {
  source: string;
  source_id: string;
  account_id: string;
  email?: string | null;
  username?: string | null;
  display_name?: string | null;
  /** The person holding it; absent = nobody holds it yet — unless
   *  `excluded` is set, which is a different fact entirely. */
  person?: PersonSummary | null;
  /** Deliberately excluded from person metrics (bot / CI). An operator's
   *  recorded decision — not "bound to nobody", and not an invitation to
   *  bind. Optional so an older backend reads as never-excluded. */
  excluded?: boolean;
  bound_by_operator: boolean;
}

export interface AccountSearchResponse {
  items: AccountMatch[];
  /** Pass back as `cursor` for the next page; absent on the last one. */
  next_cursor?: string | null;
}

/**
 * The observed accounts and whose each one is (`GET /resolution/accounts`).
 * The person listing answers "which person is this"; this answers the question
 * an operator arrives with when they hold an account instead. Blank `q` lists
 * every open account.
 */
export async function searchAccounts(
  q: string,
  page: PageRequest = {},
  signal?: AbortSignal
): Promise<AccountSearchResponse> {
  const res = await fetchWithAuth(
    `${BASE}/resolution/accounts?q=${encodeURIComponent(q)}${pageParams(page)}`,
    { signal }
  );
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  return (await res.json()) as AccountSearchResponse;
}

/** One decision recorded for an account, newest first on the wire. */
export interface BindingHistoryEntry {
  person_id: string;
  /** The person it pointed at, as a card when the journal knows one. */
  person?: PersonSummary | null;
  author_person_id: string;
  /** The operator who decided it; absent for automation. */
  author?: PersonSummary | null;
  /** True when a human recorded it; false = automation (seed/resolver). */
  by_operator: boolean;
  /** Verb code (`operator-bind`, …) or an automation reason. Open vocabulary. */
  reason?: string | null;
  recorded_at: string;
}

/**
 * One operator call that named this account. Separate from the binding journal
 * by design: a call can move many accounts, and only the call knows why it was
 * made.
 */
export interface AccountOperation {
  operation_id: string;
  verb: string;
  author_person_id: string;
  author?: PersonSummary | null;
  /** What the operator typed, when they typed anything. */
  comment?: string | null;
  /** Accounts the call named, this one included. */
  accounts_touched: number;
  /** What it did to THIS account: `applied` | `already_decided` | `refused`. */
  outcome?: string | null;
  recorded_at: string;
}

export interface AccountBinding {
  source: string;
  source_id: string;
  account_id: string;
  /** Absent = the account is currently bound to nobody. */
  person_id?: string | null;
  history: BindingHistoryEntry[];
  /** Operator calls naming this account, newest first. Optional so a client
   *  deployed ahead of the backend keeps working. */
  operations?: AccountOperation[];
}

/**
 * One account's current binding and its full decision trail
 * (`GET /resolution/accounts/{source}/{source_id}/{account_id}`). The read
 * never 404s: an account nobody ever observed or decided answers 200 with
 * `person_id: null` and an empty history — the state a stale shared link
 * lands on.
 */
export async function getAccountBinding(ref: {
  source: string;
  source_id: string;
  account_id: string;
}): Promise<AccountBinding> {
  const path = [ref.source, ref.source_id, ref.account_id]
    .map(encodeURIComponent)
    .join("/");
  const res = await fetchWithAuth(`${BASE}/resolution/accounts/${path}`);
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let binding: AccountBinding;
  try {
    binding = (await res.json()) as AccountBinding;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(binding.history)) {
    throw new IdentityApiError(res.status, { error: "malformed_binding" });
  }
  return binding;
}

/** A source-native account, as the correction verbs address it on the wire. */
export interface WireAccountRef {
  source: string;
  source_id: string;
  /** The wire calls the account id `id` (unlike the read shapes). */
  id: string;
}

/** What happened to one requested account. Open vocabulary. */
export interface CorrectionItemResult {
  source: string;
  source_id: string;
  account_id: string;
  /** `applied` | `already_decided` | `refused` | future skip reasons. */
  outcome: string;
}

export interface CorrectionResponse {
  applied: number;
  already_decided: number;
  items: CorrectionItemResult[];
  /** Set by `detach`: the freshly minted person the account moved to. */
  new_person_id?: string | null;
}

async function postCorrection(
  path: string,
  body: unknown
): Promise<CorrectionResponse> {
  const res = await fetchWithAuth(`${BASE}/resolution/${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const errBody = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, errBody);
  }
  let outcome: CorrectionResponse;
  try {
    outcome = (await res.json()) as CorrectionResponse;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(outcome.items)) {
    throw new IdentityApiError(res.status, { error: "malformed_correction" });
  }
  return outcome;
}

/** Attach an account to a person — also the "confirm the current binding" act. */
export async function bindAccount(args: {
  account: WireAccountRef;
  person_id: string;
  comment?: string;
}): Promise<CorrectionResponse> {
  return postCorrection("bind", {
    bindings: [{ account: args.account, person_id: args.person_id }],
    comment: args.comment ?? "",
  });
}

/** One binding per account, in one call — the wire shape `bind` always took. */
export interface WireBinding {
  account: WireAccountRef;
  person_id: string;
}

/**
 * Bind MANY accounts in one call.
 *
 * The endpoint caps a call at 1000 bindings; a caller with more sends more than
 * one call and folds the answers, since one operator decision is owed one answer.
 */
export const MAX_BINDINGS_PER_CALL = 1000;

export async function bindAccounts(args: {
  bindings: WireBinding[];
  comment?: string;
}): Promise<CorrectionResponse> {
  return postCorrection("bind", {
    bindings: args.bindings,
    comment: args.comment ?? "",
  });
}

/** Declare two persons one human: every account of `source` moves to `target`. */
export async function mergePersons(args: {
  source_person_id: string;
  target_person_id: string;
  comment?: string;
}): Promise<CorrectionResponse> {
  return postCorrection("merge", {
    source_person_id: args.source_person_id,
    target_person_id: args.target_person_id,
    comment: args.comment ?? "",
  });
}

/** Detach an account into a freshly minted person. */
export async function detachAccount(args: {
  account: WireAccountRef;
  comment?: string;
}): Promise<CorrectionResponse> {
  return postCorrection("detach", {
    account: args.account,
    comment: args.comment ?? "",
  });
}

/** Exclude an account as not a person (bot / CI / service account). */
export async function excludeAccount(args: {
  account: WireAccountRef;
  comment?: string;
}): Promise<CorrectionResponse> {
  return postCorrection("exclude", {
    account: args.account,
    comment: args.comment ?? "",
  });
}

export interface PersonSearchResponse {
  items: PersonSummary[];
  /** Pass back as `cursor` for the next page; absent on the last one. */
  next_cursor?: string | null;
}

/** One canonical roster person returned by `GET /people`. */
export interface PeopleListItem {
  person_id: string;
  display_name: string | null;
  first_name: string | null;
  last_name: string | null;
  username: string | null;
  email: string | null;
  attributes: Record<string, string>;
  manager_person_id: string | null;
}

export interface PeopleListResponse {
  items: PeopleListItem[];
  next_cursor?: string | null;
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

function isPeopleListItem(value: unknown): value is PeopleListItem {
  if (typeof value !== "object" || value === null) return false;
  const person = value as Record<string, unknown>;
  const attributes = person.attributes;
  return (
    typeof person.person_id === "string" &&
    isNullableString(person.display_name) &&
    isNullableString(person.first_name) &&
    isNullableString(person.last_name) &&
    isNullableString(person.username) &&
    isNullableString(person.email) &&
    typeof attributes === "object" &&
    attributes !== null &&
    !Array.isArray(attributes) &&
    Object.values(attributes).every(
      (attribute) => typeof attribute === "string"
    ) &&
    isNullableString(person.manager_person_id)
  );
}

/** One page of a listing: where to resume and how many rows to ask for. */
export interface PageRequest {
  cursor?: string;
  limit?: number;
}

export interface PeopleListRequest extends PageRequest {
  q?: string;
  visibility?: "caller" | "tenant";
}

function pageParams(page: PageRequest): string {
  const params = new URLSearchParams();
  if (page.cursor) params.set("cursor", page.cursor);
  if (page.limit != null) params.set("limit", String(page.limit));
  const query = params.toString();
  return query ? `&${query}` : "";
}

/** One caller-authorized page of the canonical roster. */
export async function listPeople(
  page: PeopleListRequest = {},
  signal?: AbortSignal
): Promise<PeopleListResponse> {
  const params = new URLSearchParams();
  if (page.visibility) params.set("visibility", page.visibility);
  if (page.q) params.set("q", page.q);
  if (page.cursor) params.set("cursor", page.cursor);
  if (page.limit != null) params.set("limit", String(page.limit));
  const query = params.toString();
  const url = `${BASE}/people${query ? `?${query}` : ""}`;
  const res = signal
    ? await fetchWithAuth(url, { signal })
    : await fetchWithAuth(url);
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let listed: PeopleListResponse;
  try {
    listed = (await res.json()) as PeopleListResponse;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(listed.items) || !listed.items.every(isPeopleListItem)) {
    throw new IdentityApiError(res.status, { error: "malformed_roster" });
  }
  return listed;
}

/**
 * The operator's person listing (`GET /persons`) — tenant-wide, admin-gated.
 * Blank `q` lists everyone; terms narrow the same list, and a page is walked
 * with the cursor the previous one returned.
 */
export async function searchPersons(
  q: string,
  page: PageRequest = {},
  signal?: AbortSignal
): Promise<PersonSearchResponse> {
  const res = await fetchWithAuth(
    `${BASE}/persons?q=${encodeURIComponent(q)}${pageParams(page)}`,
    { signal }
  );
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let found: PersonSearchResponse;
  try {
    found = (await res.json()) as PersonSearchResponse;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(found.items)) {
    throw new IdentityApiError(res.status, { error: "malformed_search" });
  }
  return found;
}

/**
 * The persons the caller may see (`GET /visible-persons`) — one page, ordered by
 * the label each is shown under. Any signed-in caller may ask; the answer is
 * their own visible set, so it needs no admin role.
 */
export async function listVisiblePersons(
  page: PageRequest & { q?: string } = {}
): Promise<PersonSearchResponse> {
  const params = new URLSearchParams();
  if (page.q) params.set("q", page.q);
  if (page.cursor) params.set("cursor", page.cursor);
  if (page.limit != null) params.set("limit", String(page.limit));
  const query = params.toString();
  const res = await fetchWithAuth(
    `${BASE}/visible-persons${query ? `?${query}` : ""}`
  );
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let listed: PersonSearchResponse;
  try {
    listed = (await res.json()) as PersonSearchResponse;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(listed.items)) {
    throw new IdentityApiError(res.status, { error: "malformed_roster" });
  }
  return listed;
}

export interface PersonAccountEntry {
  source: string;
  source_id: string;
  account_id: string;
  email?: string | null;
  username?: string | null;
  bound_by_operator: boolean;
}

/** Every account currently bound to a person — the merge preview's substance. */
export async function getPersonAccounts(
  personId: string
): Promise<{ person_id: string; accounts: PersonAccountEntry[] }> {
  const res = await fetchWithAuth(
    `${BASE}/resolution/persons/${encodeURIComponent(personId)}/accounts`
  );
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let owned: { person_id: string; accounts: PersonAccountEntry[] };
  try {
    owned = (await res.json()) as typeof owned;
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!Array.isArray(owned.accounts)) {
    throw new IdentityApiError(res.status, { error: "malformed_accounts" });
  }
  return owned;
}

export class IdentityApiError extends Error {
  status: number;
  body: unknown;

  constructor(status: number, body: unknown) {
    super(`Identity API ${status}`);
    this.name = "IdentityApiError";
    this.status = status;
    this.body = body;
  }
}

function toIdentityPerson(p: PeopleListItem): IdentityPerson {
  return {
    person_id: p.person_id,
    email: p.email ?? "",
    display_name: p.display_name ?? "",
    first_name: p.first_name ?? "",
    last_name: p.last_name ?? "",
    department: p.attributes.department ?? "",
    division: p.attributes.division ?? "",
    job_title: p.attributes.job_title ?? "",
    status: p.attributes.status ?? "",
    username: p.username ?? "",
    parent_email: null,
    parent_id: null,
    parent_person_id: p.manager_person_id,
    supervisor_email: null,
    supervisor_name: null,
    subordinates: [],
  };
}

/** One caller-visible canonical roster person. */
export async function getPerson(
  personId: string,
  signal?: AbortSignal
): Promise<IdentityPerson> {
  const id = normalizePersonId(personId);
  const url = `${BASE}/people/${encodeURIComponent(id)}`;
  const res = signal
    ? await fetchWithAuth(url, { signal })
    : await fetchWithAuth(url);
  if (!res.ok) {
    const body = await res.json().catch(() => null);
    throw new IdentityApiError(res.status, body);
  }
  let person: unknown;
  try {
    person = await res.json();
  } catch {
    throw new IdentityApiError(res.status, { error: "invalid_json" });
  }
  if (!isPeopleListItem(person)) {
    throw new IdentityApiError(res.status, { error: "malformed_person" });
  }
  return toIdentityPerson(person);
}
