//! Operator identity corrections — write surface.
//!
//! Four verbs over the account-to-person binding: bind (single or bulk), merge,
//! detach, exclude. Each appends binding observations to `persons` under the
//! calling operator and journals the call in `operations`; nothing is updated or
//! deleted. Admin-gated like the rest of the operator surface.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use utoipa::ToSchema;
use uuid::Uuid;

use super::AppState;
use super::error::CorrectionError;
use super::gate::require_admin;
use crate::domain::person_card::{self, PersonCard};
use crate::domain::resolution::{self, EXCLUDED_PERSON, Target, Verb};
use crate::domain::review_queue::{self, EvidenceAccount, ItemKind, Review};
use crate::domain::seed::{KnownBinding, SourceAccountKey};
use crate::infra::db::{ops_repo, persons_repo, resolution_repo, seed_repo};
use crate::infra::identity_evidence::{
    AccountEvidence, AfterAccount, ClickHouseEvidenceReader, EvidenceSnapshot, ListedAccount,
};

const DEFAULT_ACCOUNT_SEARCH_LIMIT: u64 = 20;
const MAX_ACCOUNT_SEARCH_LIMIT: u64 = 100;

/// A trail is read, not paged: enough calls to cover any account's history
/// without letting one response grow without bound.
const MAX_ACCOUNT_OPERATIONS: u64 = 50;

/// How many accounts one bulk call may carry — a prepared matching table is
/// pasted by a human, not streamed.
const MAX_BULK_ITEMS: usize = 1_000;

/// A source-native account, as named by the caller.
///
/// Addressing by an observed value (e-mail / username) instead of the account
/// triple is the reserved extension for importing a prepared matching table:
/// the fields arrive optional, exactly one form is required per item, a value
/// resolving to zero or several active accounts is reported per item and never
/// guessed. The response already carries per-item outcomes, so adding it does
/// not change the shape of this contract.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AccountRef {
    /// Connector type, e.g. `github`.
    pub source: String,
    /// Connector instance id.
    pub source_id: Uuid,
    /// Account id within that instance.
    pub id: String,
}

impl From<&AccountRef> for SourceAccountKey {
    fn from(r: &AccountRef) -> Self {
        Self {
            source_type: r.source.clone(),
            source_id: r.source_id,
            account_id: r.id.clone(),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BindItem {
    pub account: AccountRef,
    pub person_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BindRequest {
    /// One or more bindings; a prepared matching table is submitted as one call.
    pub bindings: Vec<BindItem>,
    #[serde(default)]
    pub comment: String,
}
impl toolkit::api::api_dto::RequestApiDto for BindRequest {}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MergeRequest {
    /// The person being absorbed — its accounts move to the target.
    pub source_person_id: Uuid,
    /// The surviving person, named explicitly by the operator.
    pub target_person_id: Uuid,
    #[serde(default)]
    pub comment: String,
}
impl toolkit::api::api_dto::RequestApiDto for MergeRequest {}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AccountRequest {
    pub account: AccountRef,
    #[serde(default)]
    pub comment: String,
}
impl toolkit::api::api_dto::RequestApiDto for AccountRequest {}

/// What happened to one requested account.
#[derive(Debug, Serialize, ToSchema)]
pub struct ItemResult {
    pub source: String,
    pub source_id: Uuid,
    pub account_id: String,
    /// `applied` — the binding is in force;
    /// `already_decided` — the same operator decision was already recorded;
    /// `refused` — the write could not place the row (a concurrent operation
    /// held the key); the account keeps its previous binding.
    /// Open vocabulary: value-addressed items will report their skip reasons
    /// (`ambiguous_value`, `unknown_value`) here.
    pub outcome: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CorrectionResponse {
    pub applied: usize,
    pub already_decided: usize,
    pub items: Vec<ItemResult>,
    /// Set by `detach` when the account reached the new person; absent when
    /// the write was refused, since no binding points at that id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_person_id: Option<Uuid>,
}
impl toolkit::api::api_dto::ResponseApiDto for CorrectionResponse {}

/// `POST /v1/resolution/bind` — attach accounts to persons (also the confirm
/// act: binding an account to the person automation already gave it records the
/// operator's decision and clears it from review).
pub async fn bind(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<BindRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let operator = require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    reject_empty(req.bindings.is_empty(), "bindings")?;
    reject_oversized(req.bindings.len())?;

    let mut targets = Vec::with_capacity(req.bindings.len());
    let mut seen: HashSet<SourceAccountKey> = HashSet::with_capacity(req.bindings.len());
    for item in &req.bindings {
        let account = SourceAccountKey::from(&item.account);

        // One call naming an account twice has no answer: which person wins is
        // the caller's contradiction to resolve, not ours to guess.
        if !seen.insert(account.clone()) {
            return Err(invalid(
                "bindings",
                &format!(
                    "account {}:{} appears more than once",
                    account.source_type, account.account_id
                ),
            ));
        }

        reject_excluded_person(item.person_id, "person_id")?;
        require_known_person(&state.db, tenant, item.person_id).await?;
        targets.push(Target {
            account,
            person_id: item.person_id,
        });
    }

    let response = apply_correction(
        &state,
        tenant,
        operator,
        targets,
        Decision {
            verb: Verb::Bind,
            // Heterogeneous bulk detail is in the per-account list either way.
            target_person_id: req.bindings[0].person_id,
            comment: &req.comment,
        },
    )
    .await?;

    Ok(Json(response))
}

/// `POST /v1/resolution/merge` — declare two persons one human; every account of
/// the absorbed person is rebound to the survivor.
pub async fn merge(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<MergeRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let operator = require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    if req.source_person_id == req.target_person_id {
        return Err(invalid(
            "target_person_id",
            "source and target are the same person",
        ));
    }
    reject_excluded_person(req.source_person_id, "source_person_id")?;
    reject_excluded_person(req.target_person_id, "target_person_id")?;
    require_known_person(&state.db, tenant, req.source_person_id).await?;
    require_known_person(&state.db, tenant, req.target_person_id).await?;

    let accounts = resolution_repo::accounts_of_person(&state.db, tenant, req.source_person_id)
        .await
        .map_err(|e| internal(&e, "failed to read the person's accounts"))?;
    let targets = accounts
        .into_iter()
        .map(|account| Target {
            account,
            person_id: req.target_person_id,
        })
        .collect();

    let outcome = apply_correction(
        &state,
        tenant,
        operator,
        targets,
        Decision {
            verb: Verb::Merge,
            target_person_id: req.target_person_id,
            comment: &req.comment,
        },
    )
    .await?;

    Ok(Json(outcome))
}

/// `POST /v1/resolution/detach` — declare that an account belongs to a different
/// human; the account moves to a freshly minted person.
pub async fn detach(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<AccountRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let operator = require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let account = SourceAccountKey::from(&req.account);
    require_known_account(&state, tenant, &account).await?;

    let new_person_id = Uuid::now_v7();
    let targets = vec![Target {
        account,
        person_id: new_person_id,
    }];

    let mut outcome = apply_correction(
        &state,
        tenant,
        operator,
        targets,
        Decision {
            verb: Verb::Detach,
            target_person_id: new_person_id,
            comment: &req.comment,
        },
    )
    .await?;

    // Only name the person when the account actually reached it: a refused
    // write leaves the account where it was, and an id no binding points at
    // would read as a person that exists.
    if outcome.applied > 0 {
        outcome.new_person_id = Some(new_person_id);
    }

    Ok(Json(outcome))
}

/// `POST /v1/resolution/exclude` — mark an account as not a human (bot, CI,
/// service account). It binds to the reserved excluded person.
pub async fn exclude(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    Json(req): Json<AccountRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    let operator = require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let account = SourceAccountKey::from(&req.account);
    require_known_account(&state, tenant, &account).await?;

    let targets = vec![Target {
        account,
        person_id: EXCLUDED_PERSON,
    }];

    let outcome = apply_correction(
        &state,
        tenant,
        operator,
        targets,
        Decision {
            verb: Verb::Exclude,
            target_person_id: EXCLUDED_PERSON,
            comment: &req.comment,
        },
    )
    .await?;

    Ok(Json(outcome))
}

/// What the operator asked for, as the operations journal will record it.
/// Grouped so the two person ids in play — the author and the verb's target —
/// cannot be swapped at a call site without the compiler noticing.
struct Decision<'a> {
    verb: Verb,
    /// The verb-level person the journal names: the survivor for a merge, the
    /// minted person for a detach, the sentinel for an exclude. Named by the
    /// caller, never derived from the targets — a merge of a person with no
    /// accounts has an empty target list but still a real survivor.
    target_person_id: Uuid,
    comment: &'a str,
}

/// Read the targets' current bindings, build the rows the correction appends,
/// write them once, and journal the call — one write per operation, however
/// many accounts it names.
async fn apply_correction(
    state: &AppState,
    tenant: Uuid,
    operator: Uuid,
    targets: Vec<Target>,
    decision: Decision<'_>,
) -> Result<CorrectionResponse, CanonicalError> {
    let Decision {
        verb,
        target_person_id,
        comment,
    } = decision;

    let accounts: Vec<SourceAccountKey> = targets.iter().map(|t| t.account.clone()).collect();
    let current = resolution_repo::current_bindings(&state.db, tenant, &accounts)
        .await
        .map_err(|e| internal(&e, "failed to read current bindings"))?;

    let pairs: Vec<_> = targets
        .iter()
        .map(|t| (t, current.get(&t.account).copied()))
        .collect();
    let rows = resolution::build_rows(pairs, operator, verb, chrono::Utc::now().naive_utc());

    // Row i belongs to the target at `written[i]`: outcomes are reported by
    // position, never by account, so one call naming an account twice cannot
    // have its results collapse into one.
    let written: Vec<usize> = targets
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            rows.iter()
                .any(|r| r.account == t.account && r.person_id == t.person_id)
        })
        .map(|(index, _)| index)
        .collect();
    let landed = write_rows(state, tenant, rows).await?;

    let mut outcomes = vec![OUTCOME_ALREADY_DECIDED; targets.len()];
    for (slot, index) in written.iter().enumerate() {
        outcomes[*index] = if landed.get(slot).copied().unwrap_or(false) {
            OUTCOME_APPLIED
        } else {
            OUTCOME_REFUSED
        };
    }

    let items: Vec<ItemResult> = targets
        .iter()
        .zip(&outcomes)
        .map(|(t, outcome)| ItemResult {
            source: t.account.source_type.clone(),
            source_id: t.account.source_id,
            account_id: t.account.account_id.clone(),
            outcome: (*outcome).to_owned(),
        })
        .collect();

    journal(
        state,
        tenant,
        operator,
        verb,
        target_person_id,
        comment,
        &items,
    )
    .await;

    let applied = count_items(&items, OUTCOME_APPLIED);
    if applied > 0 {
        // Spawned: the correction is already durable in `persons`, so the
        // response must not wait on ClickHouse.
        let config = state.config.clone();
        tokio::spawn(async move { publish_correction(&config).await });
    }

    Ok(CorrectionResponse {
        applied,
        already_decided: count_items(&items, OUTCOME_ALREADY_DECIDED),
        items,
        new_person_id: None,
    })
}

/// Publish the corrected log into the snapshot the metrics resolver reads.
/// Never fails the verb: the runner waits for a busy lock, the lock holder's
/// quiescence re-check covers rows it raced with, and the next publish is
/// the catch-up path for everything else.
async fn publish_correction(config: &crate::config::GearConfig) {
    use crate::sync_runner::{self, SyncRunError};
    match sync_runner::run(config, false).await {
        Ok(outcome) => tracing::info!(?outcome, "persons-sync published the corrected log"),
        Err(SyncRunError::LockBusy) => tracing::warn!(
            "publish lock stayed busy past the wait; the correction reaches the resolver with \
             the next publish"
        ),
        Err(SyncRunError::Guard(msg)) => {
            tracing::warn!(%msg, "persons-sync refused the post-correction publish");
        }
        Err(SyncRunError::Failed(e)) => tracing::warn!(
            error = %format!("{e:#}"),
            "publishing the correction failed; the resolver snapshot is stale until the next publish"
        ),
    }
}

fn count_items(items: &[ItemResult], wanted: &str) -> usize {
    items.iter().filter(|i| i.outcome == wanted).count()
}

/// Append the rows, then recover only those the database refused.
///
/// The natural key has no account discriminator, so a concurrent operation can
/// have claimed the same microsecond and `INSERT IGNORE` silently drops the
/// loser. A short write is diagnosed by asking which of these exact rows the
/// journal now holds — author and instant included, because a confirmation
/// writes an operator row over an automatic binding to the same person and
/// "the account points at this person" cannot tell those two apart. Only the
/// rows that are missing are re-stamped and retried; the ones that landed must
/// not be sent again or the history gains duplicates.
///
/// Returns, per input row, whether its observation is in the journal.
async fn write_rows(
    state: &AppState,
    tenant: Uuid,
    rows: Vec<resolution::BindingRow>,
) -> Result<Vec<bool>, CanonicalError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let appended = append(state, tenant, &rows).await?;
    if appended == rows.len() as u64 {
        return Ok(vec![true; rows.len()]);
    }

    let mut present = present_rows(state, tenant, &rows).await?;

    let missing = resolution::missing(&rows, &present);
    if missing.is_empty() {
        return Ok(present);
    }

    let retry = resolution::restamp(&missing, chrono::Utc::now().naive_utc());
    append(state, tenant, &retry).await?;

    let recovered = present_rows(state, tenant, &retry).await?;
    resolution::apply_recovery(&mut present, &recovered);

    let refused = present.iter().filter(|landed| !**landed).count();
    if refused > 0 {
        tracing::warn!(
            refused,
            "identity correction: rows the database refused twice"
        );
    }
    Ok(present)
}

async fn append(
    state: &AppState,
    tenant: Uuid,
    rows: &[resolution::BindingRow],
) -> Result<u64, CanonicalError> {
    resolution_repo::append_bindings(&state.db, tenant, rows)
        .await
        .map_err(|e| internal(&e, "failed to append the correction"))
}

async fn present_rows(
    state: &AppState,
    tenant: Uuid,
    rows: &[resolution::BindingRow],
) -> Result<Vec<bool>, CanonicalError> {
    resolution_repo::present_rows(&state.db, tenant, rows)
        .await
        .map_err(|e| internal(&e, "failed to verify the correction"))
}

/// Record the call in the operations journal. Journalling must never fail the
/// correction — the binding is already committed and is the source of truth.
async fn journal(
    state: &AppState,
    tenant: Uuid,
    operator: Uuid,
    verb: Verb,
    target_person_id: Uuid,
    comment: &str,
    items: &[ItemResult],
) {
    let summary = serde_json::json!({
        "applied": count_items(items, OUTCOME_APPLIED),
        "already_decided": count_items(items, OUTCOME_ALREADY_DECIDED),
        "refused": count_items(items, OUTCOME_REFUSED),
    });
    let request = serde_json::json!({
        "verb": verb.reason_code(),
        "target_person_id": target_person_id,
        "comment": comment,
        "accounts": items.iter().map(|i| serde_json::json!({
            "source": i.source,
            "source_id": i.source_id,
            "account_id": i.account_id,
            "outcome": i.outcome,
        })).collect::<Vec<_>>(),
    });

    let operation_id = Uuid::now_v7();
    let journalled = async {
        ops_repo::enqueue(
            &state.db,
            operation_id,
            RESOLUTION_OP,
            tenant,
            operator,
            Some(&request.to_string()),
        )
        .await?;
        ops_repo::try_start(&state.db, operation_id).await?;
        ops_repo::complete(&state.db, operation_id, &summary.to_string()).await
    }
    .await;

    if let Err(e) = journalled {
        tracing::error!(error = %e, "identity correction: journalling failed");
    }
}

/// `operations.operation_type` for operator corrections.
pub const RESOLUTION_OP: &str = "identity-correction";

/// Per-item outcome vocabulary.
const OUTCOME_APPLIED: &str = "applied";
const OUTCOME_ALREADY_DECIDED: &str = "already_decided";
const OUTCOME_REFUSED: &str = "refused";

/// Detaching or excluding presupposes an account that exists: it must already
/// have a binding, or a connector must have observed it. (Binding an account
/// ahead of its first sync is deliberate — pre-registration — but minting a
/// person for an account nobody has seen is a typo, not a decision.)
async fn require_known_account(
    state: &AppState,
    tenant: Uuid,
    account: &SourceAccountKey,
) -> Result<(), CanonicalError> {
    let bindings =
        resolution_repo::current_bindings(&state.db, tenant, std::slice::from_ref(account))
            .await
            .map_err(|e| internal(&e, "failed to read current bindings"))?;
    if !bindings.is_empty() {
        return Ok(());
    }

    let reader = evidence_reader(state);
    let observed = reader
        .has_account(account)
        .await
        .map_err(|e| internal(&e, "failed to check connector evidence"))?;
    if observed {
        return Ok(());
    }

    Err(CorrectionError::not_found("account not found")
        .with_resource(format!(
            "{}:{}:{}",
            account.source_type, account.source_id, account.account_id
        ))
        .create())
}

async fn require_known_person(
    db: &sea_orm::DatabaseConnection,
    tenant: Uuid,
    person_id: Uuid,
) -> Result<(), CanonicalError> {
    let known = resolution_repo::person_exists(db, tenant, person_id)
        .await
        .map_err(|e| internal(&e, "failed to check the person"))?;
    if known {
        return Ok(());
    }
    Err(CorrectionError::not_found("person not found")
        .with_resource(person_id.to_string())
        .create())
}

/// The excluded person is a sentinel, not a person: its first exclusion writes
/// it into the journal, after which `person_exists` would vouch for it. As a
/// bind target it becomes an exclude that skips `require_known_account`; as a
/// merge side it moves every excluded account of the tenant in one call. Only
/// the exclude verb may name it.
fn reject_excluded_person(person_id: Uuid, field: &str) -> Result<(), CanonicalError> {
    if person_id == EXCLUDED_PERSON {
        return Err(invalid(
            field,
            "the reserved excluded person cannot take part in a correction; use the exclude verb",
        ));
    }
    Ok(())
}

fn reject_empty(is_empty: bool, field: &str) -> Result<(), CanonicalError> {
    if is_empty {
        return Err(invalid(field, "must not be empty"));
    }
    Ok(())
}

fn reject_oversized(len: usize) -> Result<(), CanonicalError> {
    if len > MAX_BULK_ITEMS {
        return Err(invalid(
            "bindings",
            &format!("at most {MAX_BULK_ITEMS} bindings per call"),
        ));
    }
    Ok(())
}

fn invalid(field: &str, message: &str) -> CanonicalError {
    CorrectionError::invalid_argument()
        .with_field_violation(field, message, "INVALID")
        .create()
}

fn internal(error: &anyhow::Error, message: &str) -> CanonicalError {
    tracing::error!(error = %error, "{message}");
    CanonicalError::internal(message).create()
}

/// The queue cannot be derived: the binding read stopped at its ceiling, so
/// whether a given account is bound is unknown rather than false. Refusing is
/// the safe answer — the alternative is a queue of accounts that look unbound
/// and are not.
fn bindings_beyond_ceiling() -> CanonicalError {
    tracing::error!(
        cap = resolution_repo::MAX_TENANT_BINDINGS,
        "review queue: the tenant holds more bindings than this surface reads; \
         refusing to classify accounts whose binding may sit past the ceiling"
    );
    CanonicalError::internal("the tenant holds more identity bindings than the review queue reads")
        .create()
}

/// Query knobs for the review queue.
#[derive(Debug, Deserialize)]
pub struct AttentionParams {
    /// Cap on returned items (rates always cover every observed account).
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct QueueItemResponse {
    /// `contested` | `binding_conflict` | `provisioned_at_login` |
    /// `minted_from_roster` | `no_source_id` | `no_evidence`.
    pub kind: String,
    pub source: String,
    pub source_id: Uuid,
    pub account_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    /// How the source describes the account. Nothing here is matchable — it is
    /// what lets an operator recognise whose account this is when automation
    /// cannot, which is exactly the case for the ones only they can bind.
    pub display_name: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub status: Option<String>,
    pub manager_email: Option<String>,
    /// Who holds the account right now. Absent = nobody, which is what an
    /// unbound account on the queue means; present, it names which of the
    /// candidates below is the one being disagreed with.
    pub bound_to: Option<Uuid>,
    /// Persons this account could belong to, if any are known — hydrated into
    /// cards so the operator UI never has to resolve bare ids itself.
    pub candidates: Vec<PersonSummaryResponse>,
}

/// A person as operator surfaces display them: enough to recognise and pick,
/// nothing more. Every field but the id may be null — a person the journal
/// knows only through bindings still appears, as the id alone.
#[derive(Debug, Serialize, ToSchema)]
pub struct PersonSummaryResponse {
    pub person_id: Uuid,
    /// The journal holds nothing but an automatic mint for this person — a
    /// sign-in that needed somebody to enter as, or a roster listing an account
    /// with no address. They may duplicate one the roster knows, so they are not
    /// a merge target: the history is on the other side.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provisional: bool,
    pub email: Option<String>,
    /// Source-native handle (e.g. a git login) — often the only recognisable
    /// field of an identity no HR system has observed yet.
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub job_title: Option<String>,
    pub status: Option<String>,
}

impl From<PersonCard> for PersonSummaryResponse {
    fn from(card: PersonCard) -> Self {
        Self {
            person_id: card.person_id,
            provisional: false,
            email: card.email,
            username: card.username,
            display_name: card.display_name,
            job_title: card.job_title,
            status: card.status,
        }
    }
}

/// Mark the cards the journal holds nothing but a login-mint for.
///
/// A separate read rather than a card attribute: the mark is a fact about the
/// person's bindings, and the card is assembled from observed values.
///
/// # Errors
///
/// Returns an error if the read fails.
pub async fn mark_provisional(
    state: &AppState,
    tenant: Uuid,
    people: &mut [PersonSummaryResponse],
) -> Result<(), CanonicalError> {
    let ids: Vec<Uuid> = people.iter().map(|p| p.person_id).collect();
    let provisional = persons_repo::provisional_persons(&state.db, tenant, &ids)
        .await
        .map_err(|e| internal(&e, "failed to read which persons are provisional"))?;

    for person in people {
        person.provisional = provisional.contains(&person.person_id);
    }
    Ok(())
}

/// How the tenant's observed accounts are split across the resolution states.
///
/// Deliberately no person total. A journal-wide count answers "how many ids have
/// we ever written", which after a merge never falls and after a detach only
/// rises — so it read as a roster size while measuring something else. A figure
/// that would have to be explained every time it is read is worse than none.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResolutionRatesResponse {
    pub observed: usize,
    pub bound: usize,
    pub pending: usize,
    pub no_source_id: usize,
    pub no_evidence: usize,
    pub excluded: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AttentionResponse {
    pub items: Vec<QueueItemResponse>,
    pub rates: ResolutionRatesResponse,
    /// The evidence read hit its safety cap: the queue and the rates describe
    /// only the first accounts of the tenant, not all of them. Consumers must
    /// not present these numbers as tenant-wide. (The binding read cannot be a
    /// prefix — a partial one would misclassify, so it fails the request.)
    pub truncated: bool,
    /// `limit` cut the item list — more accounts await a decision than are
    /// listed here. Distinct from `truncated`: the rates stay whole-tenant,
    /// only this page is short.
    pub items_truncated: bool,
}
impl toolkit::api::api_dto::ResponseApiDto for AttentionResponse {}

/// `GET /v1/resolution/attention` — what needs an operator decision, derived
/// from the evidence fold joined with current bindings, plus the match rate.
pub async fn attention(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    axum::extract::Query(params): axum::extract::Query<AttentionParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let (review, truncated) = build_review(&state, tenant).await?;

    let limit = params.limit.map_or(DEFAULT_QUEUE_LIMIT, |l| {
        usize::try_from(l).unwrap_or(1).clamp(1, MAX_QUEUE_LIMIT)
    });
    let items_truncated = review.items.len() > limit;
    let page: Vec<_> = review.items.into_iter().take(limit).collect();

    let cards = candidate_cards(state.as_ref(), tenant, &page).await?;
    // Over the candidate ids, NOT the hydrated cards' keys: a login-minted
    // person has no card attributes at all (their journal holds only the
    // binding row), so the card set misses exactly the persons this flag
    // exists to mark.
    let candidate_ids: Vec<Uuid> = page
        .iter()
        .flat_map(|i| i.candidates.iter().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let provisional = persons_repo::provisional_persons(&state.db, tenant, &candidate_ids)
        .await
        .map_err(|e| internal(&e, "failed to read which persons are provisional"))?;

    let items = page
        .into_iter()
        .map(|i| QueueItemResponse {
            kind: kind_label(i.kind).to_owned(),
            source: i.account.source_type,
            source_id: i.account.source_id,
            account_id: i.account.account_id,
            email: i.email,
            username: i.username,
            display_name: i.description.display_name,
            job_title: i.description.job_title,
            department: i.description.department,
            status: i.description.status,
            manager_email: i.description.manager_email,
            bound_to: i.bound_to,
            candidates: person_card::in_requested_order(&i.candidates, &cards)
                .into_iter()
                .map(|card| {
                    let is_provisional = provisional.contains(&card.person_id);
                    PersonSummaryResponse {
                        provisional: is_provisional,
                        ..PersonSummaryResponse::from(card)
                    }
                })
                .collect(),
        })
        .collect();

    Ok(Json(AttentionResponse {
        items,
        rates: ResolutionRatesResponse {
            observed: review.rates.observed,
            bound: review.rates.bound,
            pending: review.rates.pending,
            no_source_id: review.rates.no_source_id,
            no_evidence: review.rates.no_evidence,
            excluded: review.rates.excluded,
        },
        truncated,
        items_truncated,
    }))
}

/// One decision in an account's history.
#[derive(Debug, Serialize, ToSchema)]
pub struct HistoryEntry {
    pub person_id: Uuid,
    /// The person this decision pointed at, as a card when one is known.
    pub person: Option<PersonSummaryResponse>,
    pub author_person_id: Uuid,
    /// The operator who decided it; absent for automation, which has no card.
    pub author: Option<PersonSummaryResponse>,
    /// `true` when a person made this decision, `false` for automation.
    pub by_operator: bool,
    pub reason: Option<String>,
    pub recorded_at: String,
}

/// One operator call that named this account.
///
/// The binding journal says what a decision did; this says who ran it, the
/// reason they gave, and how far it reached — a merge lands one row here and
/// one in every other account it moved.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccountOperationResponse {
    pub operation_id: Uuid,
    /// `operator-bind` | `operator-merge` | `operator-detach` | `operator-exclude`.
    pub verb: String,
    pub author_person_id: Uuid,
    /// The author as a card, when the journal knows them by more than an id.
    pub author: Option<PersonSummaryResponse>,
    /// What the operator typed, when they typed anything.
    pub comment: Option<String>,
    /// Accounts the call named, this one included.
    pub accounts_touched: usize,
    /// What the call did to THIS account: `applied` | `already_decided` |
    /// `refused`. A refusal changed nothing and still belongs in the trail.
    pub outcome: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AccountBindingResponse {
    pub source: String,
    pub source_id: Uuid,
    pub account_id: String,
    /// The binding in force now, if the account has one.
    pub person_id: Option<Uuid>,
    pub history: Vec<HistoryEntry>,
    /// Operator calls that named this account, newest first. Absent from the
    /// binding journal by design: one call can move many accounts, and only
    /// the call knows why it was made.
    pub operations: Vec<AccountOperationResponse>,
}
impl toolkit::api::api_dto::ResponseApiDto for AccountBindingResponse {}

/// One account as a search answers for it: what it is, and whose it is.
#[derive(Debug, Serialize, ToSchema)]
pub struct AccountMatchResponse {
    pub source: String,
    pub source_id: Uuid,
    pub account_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    /// The person holding it, hydrated. Absent means nobody holds it yet —
    /// except when `excluded` is set, which is a different fact entirely.
    pub person: Option<PersonSummaryResponse>,
    /// The account is deliberately excluded from person metrics (a bot, CI, a
    /// service account). Without this an exclusion — an operator's recorded
    /// decision — would read as "bound to nobody" and invite undoing it.
    pub excluded: bool,
    /// `true` when a person decided this binding rather than automation.
    pub bound_by_operator: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AccountSearchResponse {
    pub items: Vec<AccountMatchResponse>,
    /// Pass back as `?cursor=` for the next page; absent on the last one. Only
    /// valid for the query that issued it — narrowing `q` starts over.
    pub next_cursor: Option<String>,
}
impl toolkit::api::api_dto::ResponseApiDto for AccountSearchResponse {}

#[derive(Debug, Deserialize)]
pub struct AccountSearchParams {
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// Where the next page of accounts resumes. Opaque on the wire.
#[derive(Debug, Serialize, Deserialize)]
struct AccountPageKey {
    order_label: String,
    source: String,
    source_id: String,
    account_id: String,
}

impl super::listing::PagePosition for AccountPageKey {
    const KIND: &'static str = "observed-accounts";
}

/// `GET /v1/resolution/accounts` — list the observed accounts and say whose
/// each one is; `?q=` narrows the same list.
///
/// The person listing answers "which person is this"; this answers the question
/// an operator arrives with when they hold an account instead: a git handle
/// from a review, an address from a ticket. Browsing it is bounded by the page,
/// so an operator can also just look at what the connectors reported.
pub async fn search_accounts(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    axum::extract::Query(params): axum::extract::Query<AccountSearchParams>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let needle = params.q.unwrap_or_default().trim().to_owned();
    let limit = super::listing::clamp_limit(
        params.limit,
        DEFAULT_ACCOUNT_SEARCH_LIMIT,
        MAX_ACCOUNT_SEARCH_LIMIT,
    );
    let resume = account_resume_from(params.cursor.as_deref(), tenant, &needle)?;

    let reader = evidence_reader(&state);
    let page = reader
        .list(
            (!needle.is_empty()).then_some(needle.as_str()),
            resume.as_ref().map(|key| AfterAccount {
                order_label: &key.order_label,
                source_type: &key.source,
                source_id: &key.source_id,
                account_id: &key.account_id,
            }),
            limit,
        )
        .await
        .map_err(|e| internal(&e, "failed to read the observed accounts"))?;

    let next_cursor = page
        .more
        .then(|| next_account_cursor(page.accounts.last(), tenant, &needle))
        .transpose()?
        .flatten();

    let found: Vec<AccountEvidence> = page.accounts.into_iter().map(|row| row.evidence).collect();
    let keys: Vec<SourceAccountKey> = found.iter().map(|e| e.account.clone()).collect();
    let bindings = resolution_repo::current_bindings(&state.db, tenant, &keys)
        .await
        .map_err(|e| internal(&e, "failed to read current bindings"))?;

    let person_ids: Vec<Uuid> = bindings
        .values()
        .map(|b| b.person_id)
        .filter(|id| *id != EXCLUDED_PERSON)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let cards = persons_repo::person_cards(&state.db, tenant, &person_ids)
        .await
        .map_err(|e| internal(&e, "failed to hydrate the holders"))?;
    let provisional = persons_repo::provisional_persons(&state.db, tenant, &person_ids)
        .await
        .map_err(|e| internal(&e, "failed to read which persons are provisional"))?;

    let items = found
        .into_iter()
        .map(|account| {
            let binding = bindings.get(&account.account);
            AccountMatchResponse {
                source: account.account.source_type,
                source_id: account.account.source_id,
                account_id: account.account.account_id,
                email: account.email,
                username: account.username,
                display_name: account.description.display_name,
                // A holder with no card attributes (a login-minted stub) is
                // still a holder: back-fill an id-only card rather than
                // presenting a bound account as unbound.
                person: binding.filter(|b| b.person_id != EXCLUDED_PERSON).map(|b| {
                    let card = cards
                        .get(&b.person_id)
                        .cloned()
                        .unwrap_or_else(|| PersonCard::empty(b.person_id));
                    PersonSummaryResponse {
                        provisional: provisional.contains(&b.person_id),
                        ..PersonSummaryResponse::from(card)
                    }
                }),
                excluded: binding.is_some_and(|b| b.person_id == EXCLUDED_PERSON),
                bound_by_operator: binding.is_some_and(KnownBinding::is_operator_authored),
            }
        })
        .collect();

    Ok(Json(AccountSearchResponse { items, next_cursor }))
}

fn account_resume_from(
    cursor: Option<&str>,
    tenant: Uuid,
    needle: &str,
) -> Result<Option<AccountPageKey>, CanonicalError> {
    let Some(cursor) = cursor.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };

    super::listing::decode_cursor::<AccountPageKey>(cursor, tenant, needle)
        .map(Some)
        .map_err(|rejected| invalid("cursor", rejected.message()))
}

/// The cursor that resumes after the last row served.
fn next_account_cursor(
    last: Option<&ListedAccount>,
    tenant: Uuid,
    needle: &str,
) -> Result<Option<String>, CanonicalError> {
    last.map(|row| {
        super::listing::encode_cursor(
            tenant,
            needle,
            &AccountPageKey {
                order_label: row.order_label.clone(),
                source: row.evidence.account.source_type.clone(),
                source_id: row.evidence.account.source_id.to_string(),
                account_id: row.evidence.account.account_id.clone(),
            },
        )
    })
    .transpose()
    .map_err(|e| {
        tracing::error!(error = %e, "failed to issue an account page cursor");
        CanonicalError::internal("failed to list the observed accounts").create()
    })
}

/// `GET /v1/resolution/accounts/{source}/{source_id}/{account_id}` — why this
/// account belongs to this person: the binding in force and every decision
/// behind it.
pub async fn account_binding(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    axum::extract::Path((source, source_id, account_id)): axum::extract::Path<(
        String,
        Uuid,
        String,
    )>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let account = SourceAccountKey {
        source_type: source.clone(),
        source_id,
        account_id: account_id.clone(),
    };
    let history = resolution_repo::binding_history(&state.db, tenant, &account)
        .await
        .map_err(|e| internal(&e, "failed to read the binding history"))?;

    let operations = ops_repo::corrections_for_account(
        &state.db,
        tenant,
        RESOLUTION_OP,
        &source,
        source_id,
        &account_id,
        MAX_ACCOUNT_OPERATIONS,
    )
    .await
    .map_err(|e| internal(&e, "failed to read the account's operations"))?;

    // One hydration for every person the trail names — the people it points at
    // and the operators who decided — so no consumer is left resolving ids.
    let named: Vec<Uuid> = history
        .iter()
        .flat_map(|h| [h.person_id, h.author_person_id])
        .chain(operations.iter().map(|o| o.author_person_id))
        .filter(|id| !id.is_nil())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let cards = persons_repo::person_cards(&state.db, tenant, &named)
        .await
        .map_err(|e| internal(&e, "failed to read the persons named in the trail"))?;
    let card_of = |id: Uuid| cards.get(&id).cloned().map(PersonSummaryResponse::from);

    let entries: Vec<HistoryEntry> = history
        .iter()
        .map(|h| HistoryEntry {
            person_id: h.person_id,
            person: card_of(h.person_id),
            author_person_id: h.author_person_id,
            author: card_of(h.author_person_id),
            by_operator: !h.author_person_id.is_nil(),
            reason: h.reason.clone(),
            recorded_at: super::seed::fmt_ts(h.created_at),
        })
        .collect();

    let calls = operations
        .iter()
        .map(|op| account_operation(op, &account, card_of(op.author_person_id)))
        .collect();

    Ok(Json(AccountBindingResponse {
        source,
        source_id,
        account_id,
        person_id: history.first().map(|h| h.person_id),
        history: entries,
        operations: calls,
    }))
}

/// Read one correction call as the trail shows it, from the payload it stored.
fn account_operation(
    op: &ops_repo::Operation,
    account: &SourceAccountKey,
    author: Option<PersonSummaryResponse>,
) -> AccountOperationResponse {
    let request: serde_json::Value = op
        .request_json
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(serde_json::Value::Null);
    let accounts = request
        .get("accounts")
        .and_then(serde_json::Value::as_array)
        .map_or(&[][..], Vec::as_slice);

    AccountOperationResponse {
        operation_id: op.operation_id,
        verb: request
            .get("verb")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        author_person_id: op.author_person_id,
        author,
        comment: request
            .get("comment")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_owned),
        accounts_touched: accounts.len(),
        outcome: accounts
            .iter()
            .find(|entry| names_account(entry, account))
            .and_then(|entry| entry.get("outcome"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        recorded_at: super::seed::fmt_ts(op.started_at),
    }
}

fn names_account(entry: &serde_json::Value, account: &SourceAccountKey) -> bool {
    entry.get("source").and_then(serde_json::Value::as_str) == Some(&account.source_type)
        && entry.get("account_id").and_then(serde_json::Value::as_str) == Some(&account.account_id)
        && entry.get("source_id").and_then(serde_json::Value::as_str)
            == Some(&account.source_id.to_string())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PersonAccountEntry {
    pub source: String,
    pub source_id: Uuid,
    pub account_id: String,
    pub email: Option<String>,
    pub username: Option<String>,
    /// `true` when the account's current binding was made by a person.
    pub bound_by_operator: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PersonAccountsResponse {
    pub person_id: Uuid,
    pub accounts: Vec<PersonAccountEntry>,
}
impl toolkit::api::api_dto::ResponseApiDto for PersonAccountsResponse {}

/// `GET /v1/resolution/persons/{person_id}/accounts` — the matching table for
/// one person: every account bound to them, with the values behind each link.
pub async fn person_accounts(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    axum::extract::Path(person_id): axum::extract::Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state.db, &ctx).await?;
    let tenant = ctx.subject_tenant_id();

    let accounts = resolution_repo::accounts_of_person(&state.db, tenant, person_id)
        .await
        .map_err(|e| internal(&e, "failed to read the person's accounts"))?;
    let bindings = resolution_repo::current_bindings(&state.db, tenant, &accounts)
        .await
        .map_err(|e| internal(&e, "failed to read current bindings"))?;
    let evidence = read_evidence(&state).await?;
    let by_account: HashMap<&SourceAccountKey, &AccountEvidence> =
        evidence.accounts.iter().map(|e| (&e.account, e)).collect();

    let entries = accounts
        .iter()
        .map(|account| {
            let observed = by_account.get(account).copied();
            PersonAccountEntry {
                source: account.source_type.clone(),
                source_id: account.source_id,
                account_id: account.account_id.clone(),
                email: observed.and_then(|e| e.email.clone()),
                username: observed.and_then(|e| e.username.clone()),
                bound_by_operator: bindings
                    .get(account)
                    .is_some_and(crate::domain::seed::KnownBinding::is_operator_authored),
            }
        })
        .collect();

    Ok(Json(PersonAccountsResponse {
        person_id,
        accounts: entries,
    }))
}

const DEFAULT_QUEUE_LIMIT: usize = 100;
const MAX_QUEUE_LIMIT: usize = 1_000;

fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Contested => "contested",
        ItemKind::BindingConflict => "binding_conflict",
        ItemKind::ProvisionedAtLogin => "provisioned_at_login",
        ItemKind::MintedFromRoster => "minted_from_roster",
        ItemKind::NoSourceId => "no_source_id",
        ItemKind::NoEvidence => "no_evidence",
    }
}

/// Join folded evidence with current bindings into the review, keeping the
/// read's own honesty flag: `truncated` means the rates cover a prefix of the
/// tenant, not all of it.
async fn build_review(state: &AppState, tenant: Uuid) -> Result<(Review, bool), CanonicalError> {
    let evidence = read_evidence(state).await?;
    // The whole tenant rather than the observed accounts by name: this asks
    // about all of them, and a statement's cost grows faster than the number it
    // names. Bindings for accounts no connector reports any more come back too;
    // the review only ever looks up the keys it was given.
    let bindings = resolution_repo::current_bindings_in_tenant(
        &state.db,
        tenant,
        resolution_repo::Ceiling::Bounded(resolution_repo::MAX_TENANT_BINDINGS),
    )
    .await
    .map_err(|e| internal(&e, "failed to read current bindings"))?;
    // A prefix of the bindings is not a partial answer, it is a wrong one: an
    // account whose binding sits past the ceiling reads as unbound, and the queue
    // would offer an operator a correction that overwrites a binding nobody saw.
    // The evidence half can be a prefix honestly — it decides which accounts are
    // examined, not what is true of them.
    if bindings.truncated {
        return Err(bindings_beyond_ceiling());
    }
    let truncated = evidence.truncated;

    let observed = evidence
        .accounts
        .into_iter()
        .map(|e| EvidenceAccount {
            account: e.account,
            email: e.email,
            username: e.username,
            description: e.description,
            is_closed: e.is_closed,
            states_binding_id: e.states_binding_id,
        })
        .collect();

    // The seed's own map, not one derived from the evidence: an address stays
    // claimed after the account that carried it closes, and the seed still
    // attaches new accounts to that person. See `review_queue::build`.
    let claimed_addresses = seed_repo::latest_email_to_person(&state.db, tenant)
        .await
        .map_err(|e| internal(&e, "failed to read claimed addresses"))?;

    Ok((
        review_queue::build(observed, &bindings.by_account, &claimed_addresses),
        truncated,
    ))
}

/// One hydration read for a queue page: every distinct candidate id, fetched
/// and collapsed into cards.
async fn candidate_cards(
    state: &AppState,
    tenant: Uuid,
    page: &[review_queue::QueueItem],
) -> Result<HashMap<Uuid, PersonCard>, CanonicalError> {
    let ids: Vec<Uuid> = page
        .iter()
        .flat_map(|i| i.candidates.iter().copied())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    persons_repo::person_cards(&state.db, tenant, &ids)
        .await
        .map_err(|e| internal(&e, "failed to read candidate cards"))
}

pub(super) fn evidence_reader(state: &AppState) -> ClickHouseEvidenceReader {
    ClickHouseEvidenceReader::connect(
        &state.config.clickhouse_url,
        &state.config.clickhouse_database,
        &state.config.clickhouse_user,
        &state.config.clickhouse_password,
    )
}

async fn read_evidence(state: &AppState) -> Result<EvidenceSnapshot, CanonicalError> {
    evidence_reader(state)
        .accounts()
        .await
        .map_err(|e| internal(&e, "failed to read connector evidence"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_queue_kind_has_the_wire_label_its_consumers_switch_on() {
        // The console groups by this exact string and drops anything it does not
        // know into a catch-all "needs review" bucket, so a typo here is not a
        // broken response — it is a queue that quietly stops explaining itself.
        for (kind, expected) in [
            (ItemKind::Contested, "contested"),
            (ItemKind::BindingConflict, "binding_conflict"),
            (ItemKind::ProvisionedAtLogin, "provisioned_at_login"),
            (ItemKind::MintedFromRoster, "minted_from_roster"),
            (ItemKind::NoSourceId, "no_source_id"),
            (ItemKind::NoEvidence, "no_evidence"),
        ] {
            assert_eq!(kind_label(kind), expected, "wrong label for {kind:?}");
        }
    }
}
