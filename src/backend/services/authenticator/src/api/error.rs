use toolkit_canonical_errors::resource_error;

#[resource_error("gts.cf.insight.authenticator.oidc.v1~")]
pub struct OidcError;

#[resource_error("gts.cf.insight.authenticator.service_token.v1~")]
pub struct ServiceTokenError;

#[resource_error("gts.cf.insight.authenticator.session.v1~")]
pub struct SessionError;
