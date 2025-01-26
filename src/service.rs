use crate::store::IdempotentStore;
use crate::store::RedisStore;
use axum::body::{to_bytes, Body};
use axum::extract::Request;
use axum::response::Response;
use sha2::{Digest, Sha384};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tower_layer::Layer;
use tower_service::Service;

/// A user of this library should manually add this as an extension
/// in a middleware/layer run before the IdempotentLayer
pub struct IdempotentUserId(i64);

pub async fn hash_request(req: Request) -> (Request, Vec<u8>) {
    let mut hasher = Sha384::new();

    let user_id = req.extensions().get::<IdempotentUserId>();
    // Hash authenticated user if present
    if let Some(id) = user_id {
        hasher = hasher.chain_update(&id.0.to_le_bytes());
    }

    hasher = hasher.chain_update(req.method().as_str().as_bytes());
    hasher = hasher.chain_update(req.uri().path().as_bytes());

    let (parts, body) = req.into_parts();
    // We do not care
    let bytes = to_bytes(body, usize::MAX).await.unwrap();
    hasher = hasher.chain_update(&bytes);

    let req = Request::from_parts(parts, Body::from(bytes));
    (req, hasher.finalize().to_vec())
}

/// Serialize
async fn response_to_bytes(res: Response<Body>) -> (Response, Vec<u8>) {
    let (parts, body) = res.into_parts();

    let body_bytes = to_bytes(body, usize::MAX).await.unwrap();

    let mut result = Vec::new();

    // Serialize status code
    result.extend_from_slice(&parts.status.as_u16().to_be_bytes());

    // Serialize headers
    for (name, value) in parts.headers.iter() {
        result.extend_from_slice(name.as_str().as_bytes());
        result.extend_from_slice(b": ");
        result.extend_from_slice(value.as_bytes());
        result.extend_from_slice(b"\r\n");
    }

    // Add a separator between headers and body
    result.extend_from_slice(b"\r\n");

    // Add the body bytes
    result.extend_from_slice(&body_bytes);

    (Response::from_parts(parts, Body::from(body_bytes)), result)
}

pub struct IdempotentService<S, T: IdempotentStore = RedisStore> {
    inner: S,
    store: Arc<T>,
    expire_after: Duration,
}

impl<S, T> IdempotentService<S, T>
where
    T: IdempotentStore,
{
    pub const fn new(inner: S, store: Arc<T>, expire_after: Duration) -> Self {
        IdempotentService {
            inner,
            store,
            expire_after,
        }
    }
}

impl<S, T> Service<Request> for IdempotentService<S, T>
where
    S: Service<Request, Response = Response> + Clone + 'static,
    S::Future: 'static,
    T: IdempotentStore,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let expire_after = self.expire_after;
        let store = self.store.clone();

        let mut inner = self.inner.clone();

        Box::pin(async move {
            let (req, hash) = hash_request(req).await;
            let fut = inner.call(req);
            let res = fut.await?;
            let (res, response_bytes) = response_to_bytes(res).await;
            let _ = store.set(hash, response_bytes, expire_after).await;

            Ok(res)
        })
    }
}

pub struct IdempotentLayer<T: IdempotentStore> {
    store: Arc<T>,
    expire_after: Duration,
}

impl<T> IdempotentLayer<T>
where
    T: IdempotentStore,
{
    pub const fn new(store: Arc<T>, expire_after: Duration) -> Self {
        IdempotentLayer {
            store,
            expire_after,
        }
    }
}

impl<S, T> Layer<S> for IdempotentLayer<T>
where
    T: IdempotentStore,
{
    type Service = IdempotentService<S, T>;

    fn layer(&self, service: S) -> Self::Service {
        IdempotentService::new(service, self.store.clone(), self.expire_after)
    }
}
