//! `GET /v1/datasets` and `GET /v1/datasets/{key}` — what a query may be built over.

use axum::Json;
use axum::extract::Path;
use toolkit_canonical_errors::CanonicalError;

use super::error::DatasetError;
use crate::domain::query::datasets;
use crate::domain::query::datasets::describe::{
    DatasetDescription, DatasetListDescription, describe, describe_all,
};

// INVARIANT: declarations are installation-wide, so neither handler reads the
// session's tenant.
pub async fn list_datasets() -> Result<Json<DatasetListDescription>, CanonicalError> {
    let declared = datasets::product_datasets().map_err(|error| {
        tracing::error!(error = %error, "the shipped dataset declarations are unusable");
        CanonicalError::internal("failed to list the datasets").create()
    })?;

    Ok(Json(describe_all(declared)))
}

pub async fn get_dataset(
    Path(key): Path<String>,
) -> Result<Json<DatasetDescription>, CanonicalError> {
    let declared = datasets::dataset(&key).ok_or_else(|| {
        DatasetError::not_found("no such dataset")
            .with_resource(&key)
            .create()
    })?;

    Ok(Json(describe(declared)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use toolkit_canonical_errors::Problem;

    use super::*;

    #[tokio::test]
    async fn a_key_this_build_declares_no_dataset_for_is_a_not_found_naming_it() {
        let refusal = get_dataset(Path("git_tags".to_owned()))
            .await
            .expect_err("an undeclared key is refused");
        let problem =
            serde_json::to_value(Problem::from(refusal)).expect("the envelope serializes");

        assert_eq!(problem["status"], 404);
        assert_eq!(
            problem["context"]["resource_type"],
            "gts.cf.insight.analytics_api.dataset.v1~"
        );
        assert_eq!(problem["context"]["resource_name"], "git_tags");
    }

    #[tokio::test]
    async fn a_declared_key_answers_the_description_the_listing_carries() {
        let Json(listing) = list_datasets().await.expect("the declarations load");
        let Json(one) = get_dataset(Path("git_commits".to_owned()))
            .await
            .expect("git_commits is declared");

        assert_eq!(
            listing
                .datasets
                .iter()
                .find(|dataset| dataset.key == "git_commits"),
            Some(&one),
            "the listing and the detail describe the dataset differently"
        );
    }
}
