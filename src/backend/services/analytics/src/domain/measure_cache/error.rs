use crate::domain::compiler::error::CompileError;

#[derive(Debug, thiserror::Error)]
pub enum CacheRefreshError {
    #[error("the policy store is unreadable: {0}")]
    PolicyStore(#[from] sea_orm::DbErr),
    #[error("measure `{measure}` does not compile into a cache build: {source}")]
    Uncompilable {
        measure: String,
        #[source]
        source: CompileError,
    },
    #[error("measure `{measure}` reads dataset `{dataset}`, which the catalog does not carry")]
    UncataloguedDataset { measure: String, dataset: String },
    #[error("the warehouse refused the build for measure `{measure}`")]
    BuildRefused { measure: String },
}
