use thiserror::Error;

#[derive(Debug, Error)]
pub enum CntxError {
    #[error("endpoint `{0}` was not found")]
    EndpointNotFound(String),

    #[error("no primary endpoint is configured; add one with `cntx endpoint --new`")]
    MissingPrimaryEndpoint,

    #[error("provider `{0}` requires an API key or API key environment variable")]
    MissingApiKey(String),

    #[error("model `{0}` was not found in refreshed model cache for endpoint `{1}`")]
    ModelNotFound(String, String),

    #[error("alias `{0}` already exists; remove it first or choose a different alias")]
    AliasExists(String),

    #[error("unsupported provider response shape from `{0}`")]
    UnsupportedProviderResponse(String),
}
