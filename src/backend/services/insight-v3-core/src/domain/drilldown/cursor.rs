use std::fmt::Write as _;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use toolkit_security::crypto::hash::Sha256;

use super::{Column, Sort};
use crate::domain::query::time_window::Window;

const VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Envelope {
    version: u8,
    pub(super) fingerprint: String,
    pub(super) snapshot: String,
    pub(super) window: Window,
    pub(super) key: CursorKey,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct CursorKey {
    pub(super) flag: bool,
    pub(super) value: KeyValue,
    pub(super) ties: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum KeyValue {
    Number(f64),
    Text(String),
}

#[derive(Debug, Error)]
pub(crate) enum CursorError {
    #[error("the cursor is malformed")]
    Malformed,
    #[error("the cursor was issued by a version of this service that ordered rows differently")]
    Version,
    #[error("the cursor was issued over another metric, window, order or set of columns")]
    Selection,
}

pub(super) fn encode(fingerprint: &str, snapshot: &str, window: &Window, key: CursorKey) -> String {
    let envelope = Envelope {
        version: VERSION,
        fingerprint: fingerprint.to_owned(),
        snapshot: snapshot.to_owned(),
        window: window.clone(),
        key,
    };
    let bytes = serde_json::to_vec(&envelope).unwrap_or_default();

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(super) fn decode(written: &str) -> Result<Envelope, CursorError> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(written)
        .map_err(|_| CursorError::Malformed)?;
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| CursorError::Malformed)?;
    if envelope.version != VERSION {
        return Err(CursorError::Version);
    }

    Ok(envelope)
}

pub(super) fn fingerprint(
    name: &str,
    body: &serde_json::Value,
    range: Option<&str>,
    bucket: Option<bool>,
    window: &Window,
    sort: &Sort,
    columns: &[Column],
) -> String {
    let bytes =
        serde_json::to_vec(&(name, body, range, bucket, window, sort, columns)).unwrap_or_default();

    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
