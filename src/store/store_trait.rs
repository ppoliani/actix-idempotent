use serde::de::StdError;
use std::fmt::Debug;
use std::future::Future;
use std::time::Duration;

#[derive(thiserror::Error, Debug)]
#[error("{0}")]
pub struct Error(Box<dyn StdError + Send + Sync>);

#[cfg(feature = "redis-store")]
impl From<fred::error::Error> for Error {
    fn from(err: fred::error::Error) -> Self {
        Self(Box::new(err))
    }
}

pub trait IdempotentStore: Clone + Send + Sync + 'static {
    fn get(
        &self,
        idempontent_key: Vec<u8>,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, Error>> + Send;

    fn set(
        &self,
        idempontent_key: Vec<u8>,
        body: Vec<u8>,
        expire_after: Duration,
    ) -> impl Future<Output = Result<bool, Error>> + Send;
}
