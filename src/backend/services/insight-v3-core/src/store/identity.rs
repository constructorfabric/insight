//! Identity client: whether the caller holds the admin role.
//!
//! The custom surfaces are admin-only, and roles live in the identity
//! service — the gateway JWT carries a subject and a tenant, not roles. So the
//! caller's own authorization is forwarded to `GET /v1/me` and the answer read
//! off the roles it returns, exactly as the analytics gear does it.

use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

use crate::domain::access::PersonId;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The seeded `admin` role id — a stable migration constant of the identity
/// service, mirrored here so a role check needs no extra round trip.
const ADMIN_ROLE_ID: Uuid = Uuid::from_u128(0xa4d1_1000_0000_4000_8000_0000_0000_0001);

/// The most assignments one listing answers; identity offers no next page.
const ASSIGNMENTS_PAGE: usize = 500;

/// Who a harness's fixed answer says is asking.
#[cfg(test)]
const FIXED_CALLER: Uuid = Uuid::nil();

#[derive(Debug, Deserialize)]
struct MeResponse {
    person_id: Uuid,
    roles: Vec<MeRole>,
}

#[derive(Debug, Deserialize)]
struct MeRole {
    role_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct AssignmentList {
    items: Vec<Assignment>,
}

#[derive(Debug, Deserialize)]
struct Assignment {
    person_id: Uuid,
}

#[derive(Debug, Clone)]
pub(crate) struct IdentityClient {
    mode: Mode,
}

#[derive(Debug, Clone)]
enum Mode {
    Live {
        http: reqwest::Client,
        base_url: String,
    },
    /// A fixed answer, so a handler test can be a caller with or without the
    /// role without standing up an identity service.
    #[cfg(test)]
    Fixed(Fixed),
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct Fixed {
    /// The caller when the request names none; none refuses every caller.
    caller: Option<Uuid>,
    roster: Roster,
}

/// The administrators a fixed answer lists.
#[cfg(test)]
#[derive(Debug, Clone)]
enum Roster {
    Whole(Vec<Uuid>),
    /// Listed as a full page, so possibly cut short.
    CutShort(Vec<Uuid>),
    /// Every look at who holds the role fails.
    Unreachable,
}

#[cfg(test)]
impl Roster {
    fn holds(&self, person: PersonId) -> Result<bool, IdentityError> {
        match self {
            Self::Whole(people) | Self::CutShort(people) => Ok(people.contains(&person.uuid())),
            Self::Unreachable => Err(IdentityError::Refused(503)),
        }
    }
}

impl IdentityClient {
    pub(crate) fn new(base_url: &str) -> Result<Self, IdentityError> {
        Ok(Self {
            mode: Mode::Live {
                http: reqwest::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .build()?,
                base_url: base_url.trim_end_matches('/').to_owned(),
            },
        })
    }

    /// A fixed answer. A request whose authorization is `Bearer <uuid>` is
    /// that person asking, so one harness can be several administrators.
    #[cfg(test)]
    pub(crate) fn fixed(is_admin: bool) -> Self {
        Self {
            mode: Mode::Fixed(Fixed {
                caller: is_admin.then_some(FIXED_CALLER),
                roster: Roster::Whole(vec![FIXED_CALLER]),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn admin(caller: Uuid) -> Self {
        Self {
            mode: Mode::Fixed(Fixed {
                caller: Some(caller),
                roster: Roster::Whole(vec![caller]),
            }),
        }
    }

    /// The same fixed answer, listing exactly these administrators.
    #[cfg(test)]
    pub(crate) fn with_administrators(self, people: &[Uuid]) -> Self {
        self.with_roster(Roster::Whole(people.to_vec()))
    }

    /// The same fixed answer, listing these administrators as a full page.
    #[cfg(test)]
    pub(crate) fn with_administrators_cut_short(self, people: &[Uuid]) -> Self {
        self.with_roster(Roster::CutShort(people.to_vec()))
    }

    /// The same fixed answer, failing every look at who holds the role.
    #[cfg(test)]
    pub(crate) fn with_administrators_unreachable(self) -> Self {
        self.with_roster(Roster::Unreachable)
    }

    #[cfg(test)]
    fn with_roster(self, roster: Roster) -> Self {
        match self.mode {
            Mode::Fixed(fixed) => Self {
                mode: Mode::Fixed(Fixed { roster, ..fixed }),
            },
            Mode::Live { .. } => self,
        }
    }

    pub(crate) fn is_configured(&self) -> bool {
        match &self.mode {
            Mode::Live { base_url, .. } => !base_url.is_empty(),
            #[cfg(test)]
            Mode::Fixed(_) => true,
        }
    }

    /// The caller behind `authorization` when they hold the admin role, and
    /// nothing when they do not.
    ///
    /// Who asked is answered here rather than left to the caller's word: a
    /// removal takes a dataset's records with it, so "who removed this" has
    /// to have an answer.
    pub(crate) async fn admin_caller(
        &self,
        authorization: Option<&str>,
    ) -> Result<Option<PersonId>, IdentityError> {
        let (http, base_url) = match &self.mode {
            Mode::Live { http, base_url } => (http, base_url),
            #[cfg(test)]
            Mode::Fixed(fixed) => {
                let named = authorization.and_then(named_in);
                return Ok(fixed
                    .caller
                    .map(|caller| PersonId::new(named.unwrap_or(caller))));
            }
        };

        let me: MeResponse = read(http, &format!("{base_url}/v1/me"), authorization).await?;
        let is_admin = me.roles.iter().any(|role| role.role_id == ADMIN_ROLE_ID);

        Ok(is_admin.then_some(PersonId::new(me.person_id)))
    }

    /// Whether `person` holds the admin role right now.
    pub(crate) async fn holds_admin(
        &self,
        authorization: Option<&str>,
        person: PersonId,
    ) -> Result<bool, IdentityError> {
        let (http, base_url) = match &self.mode {
            Mode::Live { http, base_url } => (http, base_url),
            #[cfg(test)]
            Mode::Fixed(fixed) => return fixed.roster.holds(person),
        };

        let url =
            format!("{base_url}/v1/person-roles?role={ADMIN_ROLE_ID}&person={person}&active=true");
        let listed: AssignmentList = read(http, &url, authorization).await?;

        Ok(listed
            .items
            .iter()
            .any(|assignment| assignment.person_id == person.uuid()))
    }

    /// Every person holding the admin role right now.
    ///
    /// SAFETY: a listing as long as a whole page may stop short of the rest,
    /// so it is refused as [`IdentityError::Truncated`] rather than read as
    /// everyone.
    pub(crate) async fn admins(
        &self,
        authorization: Option<&str>,
    ) -> Result<Vec<PersonId>, IdentityError> {
        let (http, base_url) = match &self.mode {
            Mode::Live { http, base_url } => (http, base_url),
            #[cfg(test)]
            Mode::Fixed(fixed) => {
                return match &fixed.roster {
                    Roster::Whole(people) => {
                        Ok(people.iter().copied().map(PersonId::new).collect())
                    }
                    Roster::CutShort(_) => Err(IdentityError::Truncated),
                    Roster::Unreachable => Err(IdentityError::Refused(503)),
                };
            }
        };

        let url = format!(
            "{base_url}/v1/person-roles?role={ADMIN_ROLE_ID}&active=true&limit={ASSIGNMENTS_PAGE}"
        );
        let listed: AssignmentList = read(http, &url, authorization).await?;

        administrators_in(&listed)
    }
}

/// The distinct people an assignment listing names, unless it may be cut
/// short.
fn administrators_in(listed: &AssignmentList) -> Result<Vec<PersonId>, IdentityError> {
    if listed.items.len() >= ASSIGNMENTS_PAGE {
        return Err(IdentityError::Truncated);
    }

    let mut people: Vec<PersonId> = listed
        .items
        .iter()
        .map(|assignment| PersonId::new(assignment.person_id))
        .collect();
    people.sort_unstable();
    people.dedup();

    Ok(people)
}

/// A JSON answer from identity, asked with the caller's own authorization.
async fn read<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
    authorization: Option<&str>,
) -> Result<T, IdentityError> {
    let request = match authorization {
        Some(value) => http.get(url).header(reqwest::header::AUTHORIZATION, value),
        None => http.get(url),
    };

    let response = request.send().await?;
    if !response.status().is_success() {
        return Err(IdentityError::Refused(response.status().as_u16()));
    }

    Ok(response.json().await?)
}

/// The person a harness request names as `Bearer <uuid>`.
#[cfg(test)]
fn named_in(authorization: &str) -> Option<Uuid> {
    let token = authorization.strip_prefix("Bearer ")?;

    Uuid::parse_str(token.trim()).ok()
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IdentityError {
    #[error("the identity service could not be reached")]
    Transport(#[from] reqwest::Error),
    #[error("the identity service answered {0}")]
    Refused(u16),
    #[error("the identity service listed a full page of administrators, which may be cut short")]
    Truncated,
}

impl IdentityError {
    /// Whether the identity service refused the CALLER rather than failing.
    ///
    /// A caller the identity service will not identify has not proved the
    /// role, which is a refusal to report as such — not a fault of ours to
    /// report as a 500.
    pub(crate) fn is_about_the_caller(&self) -> bool {
        matches!(self, Self::Refused(401 | 403))
    }
}

#[cfg(test)]
mod tests;
