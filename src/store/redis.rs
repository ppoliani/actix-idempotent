use crate::store::{Error, IdempotentStore};
use fred::clients::Pool;
use fred::interfaces::KeysInterface;
use fred::types::Expiration;
use std::time::Duration;
use std::{fmt::Debug, sync::Arc};

#[derive(Clone, Debug)]
pub struct RedisStore<C: KeysInterface + Clone + Send + Sync = Pool> {
    client: Arc<C>,
}

impl<C> RedisStore<C>
where
    C: KeysInterface + Clone + Send + Sync,
{
    pub fn new(client: Arc<C>) -> Self {
        Self { client }
    }
}

impl<C> IdempotentStore for RedisStore<C>
where
    C: KeysInterface + Clone + Send + Sync + 'static,
{
    async fn get(&self, idempontent_key: Vec<u8>) -> Result<Option<Vec<u8>>, Error> {
        let response = self
            .client
            .get::<Option<Vec<u8>>, &[u8]>(idempontent_key.as_ref())
            .await?;

        Ok(response)
    }

    async fn set(
        &self,
        idempontent_key: Vec<u8>,
        body: Vec<u8>,
        expire_after: Duration,
    ) -> Result<bool, Error> {
        let expiration = Expiration::EX(expire_after.as_secs() as i64);
        let inserted: bool = self
            .client
            .set::<bool, &[u8], &[u8]>(
                idempontent_key.as_ref(),
                body.as_slice(),
                Some(expiration),
                None,
                false,
            )
            .await?;

        Ok(inserted)
    }
}
