//! Where a page ended, as bytes the reader holds and hands back.
//!
//! A cursor is bound to what it was issued over: the metric as stored, the
//! window, the order and the columns. Any of those changing between two pages
//! would make the position it holds mean something else, so the cursor says
//! no rather than serving a page out of a result that no longer exists.

use std::fmt::Write as _;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use super::{Column, Sort};
use crate::domain::query::time_window::Window;

/// Bumped when the ordering key changes shape: an older cursor addresses a
/// page this key would not produce, and is refused rather than replayed.
const VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Envelope {
    version: u8,
    pub(super) fingerprint: String,
    pub(super) snapshot: String,
    /// The window as the first page resolved it. A named range means a
    /// different period an hour later, and a walk is over one period.
    pub(super) window: Window,
    pub(super) key: CursorKey,
}

/// The ordering key of the last row a page held, element for element as the
/// query compares it.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct CursorKey {
    /// The blank flag as the query computed it, which leads the key because
    /// it leads the order.
    pub(super) flag: bool,
    pub(super) value: KeyValue,
    /// Every other element of the key, as text, in key order.
    pub(super) ties: Vec<String>,
}

/// The sorted cell in the form the query orders it by.
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

/// Everything a page is bound to, as one digest. Any edit to the metric
/// changes what the rows are, so the body itself is in it; so is the
/// resolved window, so an envelope whose window was edited no longer
/// matches the digest it carries.
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
