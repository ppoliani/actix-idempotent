use axum::extract::Request;
use axum::response::Response;
use axum::RequestExt;
use ruts::store::SessionStore;
use ruts::Session;
use std::error::Error;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower_layer::Layer;
use tower_service::Service;

mod utils;

mod config;
pub use crate::config::IdempotentOptions;

use crate::utils::{bytes_to_response, hash_request, response_to_bytes};

/// Service that handles idempotent request processing.
#[derive(Clone, Debug)]
pub struct IdempotentService<S, T> {
    inner: S,
    config: IdempotentOptions,
    phantom: PhantomData<T>,
}

impl<S, T> IdempotentService<S, T> {
    pub const fn new(inner: S, config: IdempotentOptions) -> Self {
        IdempotentService::<S, T> {
            inner,
            config,
            phantom: PhantomData,
        }
    }
}

impl<S, T> Service<Request> for IdempotentService<S, T>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Error: Send,
    S::Future: Send + 'static,
    T: SessionStore,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(Into::into)
    }

    fn call(&mut self, mut req: Request) -> Self::Future {
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let config = self.config.clone();

        Box::pin(async move {
            // let session  = req.extensions().get::<Session>().cloned();
            let session = req
                .extract_parts::<Session<T>>()
                .await
                .map_err(|err| {
                    tracing::error!(
                        "Session layer not found in the request extensions: {:?}",
                        err
                    );
                    // (
                    //     StatusCode::INTERNAL_SERVER_ERROR,
                    //     "Session not found in the request",
                    // )
                    panic!("Session layer not found in the request extensions")
                })
                .unwrap();

            let (req, hash) = hash_request(req, &config).await;

            let cached_res = check_cached_response(&hash, &session).await;
            if let Err(err) = cached_res {
                tracing::error!(
                    "Failed to check cached response from the session store: {:?}",
                    err
                );
            } else if let Ok(Some(res)) = cached_res {
                return Ok(res);
            };

            let res = inner.call(req).await?;
            let (res, response_bytes) = response_to_bytes(res).await;

            if let Err(err) = session
                .update(&hash, &response_bytes, Some(config.expire_after_seconds))
                .await
            {
                tracing::error!("Failed to save response to the session store: {:?}", err);
            }

            Ok(res)
        })
    }
}

/// Layer to apply [`IdempotentService`] middleware in `axum`.
///
/// This layer caches responses in a session store and returns the cached response
/// for identical requests within the configured expiration time.
///
/// # Example
/// ```no_run
/// # use std::sync::Arc;
/// # use axum::Router;
/// # use axum::routing::get;
/// # use fred::clients::Client;
/// # use fred::interfaces::ClientLike;
/// # use ruts::{CookieOptions, SessionLayer};
/// # use axum_idempotent::{IdempotentLayer, IdempotentOptions};
/// # use ruts::store::redis::RedisStore;
/// # use tower_cookies::CookieManagerLayer;
///
/// # #[tokio::main]
/// # async fn main() {
/// # let client = Client::default();
/// # client.init().await.unwrap();
/// # let store = Arc::new(RedisStore::new(Arc::new(client)));
///
/// let idempotent_options = IdempotentOptions::default().expire_after(3);
/// let idempotent_layer = IdempotentLayer::<RedisStore<Client>>::new(idempotent_options);
///
/// let app = Router::new()
///     .route("/test", get(|| async { "Hello, World!"}))
///     .layer(idempotent_layer)
///     .layer(SessionLayer::new(store.clone())
///         .with_cookie_options(CookieOptions::build().name("session").max_age(10).path("/")))
///     .layer(CookieManagerLayer::new());
/// # let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
/// # axum::serve(listener, app).await.unwrap();
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct IdempotentLayer<T> {
    config: IdempotentOptions,
    phantom_data: PhantomData<T>,
}

impl<T> IdempotentLayer<T> {
    pub const fn new(config: IdempotentOptions) -> Self {
        IdempotentLayer {
            config,
            phantom_data: PhantomData,
        }
    }
}

impl<S, T> Layer<S> for IdempotentLayer<T> {
    type Service = IdempotentService<S, T>;

    fn layer(&self, service: S) -> Self::Service {
        IdempotentService::new(service, self.config.clone())
    }
}

async fn check_cached_response<T: SessionStore>(
    hash: impl AsRef<str>,
    session: &Session<T>,
) -> Result<Option<Response>, Box<dyn Error + Send + Sync>> {
    let response_bytes = session.get::<Vec<u8>>(hash.as_ref()).await?;

    let res = if let Some(bytes) = response_bytes {
        let response = bytes_to_response(bytes)?;

        Some(response)
    } else {
        None
    };

    Ok(res)
}
