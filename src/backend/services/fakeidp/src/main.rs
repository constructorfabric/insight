//! fakeidp binary — a thin wrapper over the library's [`fakeidp::run`].
//!
//! All logic lives in `lib.rs` so the integration test can drive the same
//! router in-process. This is a dev/e2e test double; see the crate docs.

#[tokio::main]
async fn main() {
    fakeidp::run().await;
}
