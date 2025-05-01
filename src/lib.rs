//! Middleware for handling idempotent requests in axum applications.
//!
//! This crate provides middleware that ensures idempotency of HTTP requests by caching responses
//! in a session store. When an identical request is made within the configured time window,
//! the cached response is returned instead of executing the request again.
//!
//! # Features
//!
//! - Request deduplication based on method, path, headers, and body
//! - Configurable response caching duration
//! - Header filtering options to exclude specific headers from idempotency checks
//! - Integration with session-based storage (via `ruts`)
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use axum::{Router, routing::post};
//! use fred::clients::Client;
//! use fred::interfaces::ClientLike;
//! use ruts::{CookieOptions, SessionLayer};
//! use axum_idempotent::{IdempotentLayer, IdempotentOptions};
//! use ruts::store::redis::RedisStore;
//! use tower_cookies::CookieManagerLayer;
//!
//! # #[tokio::main]
//! # async fn main() {
//! # let client = Client::default();
//! # client.init().await.unwrap();
//! # let store = Arc::new(RedisStore::new(Arc::new(client)));
//!
//! // Configure the idempotency layer
//! let idempotent_options = IdempotentOptions::default()
//!     .expire_after(60)  // Cache responses for 60 seconds
//!     .ignore_header("x-request-id".parse().unwrap());
//!
//! // Create the router with idempotency middleware
//! let app = Router::new()
//!     .route("/payments", post(process_payment))
//!     .layer(IdempotentLayer::<RedisStore<Client>>::new(idempotent_options))
//!     .layer(SessionLayer::new(store)
//!         .with_cookie_options(CookieOptions::build()
//!             .name("session")
//!             .max_age(3600)
//!             .path("/")))
//!     .layer(CookieManagerLayer::new());
//!
//! # let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
//! # axum::serve(listener, app).await.unwrap();
//! # }
//! #
//! # async fn process_payment() -> &'static str {
//! #     "Payment processed"
//! # }
//! ```
//!
//! # How it Works
//!
//! 1. When a request is received, a hash is generated from the request method, path, headers (configurable),
//!    and body.
//! 2. If an identical request (same hash) is found in the session store and hasn't expired:
//!    - The cached response is returned
//!    - The original handler is not called
//! 3. If no cached response is found:
//!    - The request is processed normally
//!    - The response is cached in the session store
//!    - The response is returned to the client
//!
//! This ensures that retrying the same request (e.g., due to network issues or client retries)
//! won't result in the operation being performed multiple times.

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
            let session = match req.extract_parts::<Session<T>>().await {
                Ok(session) => session,
                Err(err) => {
                    tracing::error!("Failed to extract Session from request: {:?}", err);
                    // Forward the request to the inner service without idempotency
                    return inner.call(req).await;
                }
            };

            let (req, hash) = hash_request(req, &config).await;

            match check_cached_response(&hash, &session).await {
                Ok(Some(res)) => return Ok(res),
                Ok(None) => {}  // No cached response, continue
                Err(err) => {
                    tracing::error!("Failed to check idempotent cached response: {:?}", err);
                    // Continue without cache
                }
            }

            let res = inner.call(req).await?;
            let (res, response_bytes) = response_to_bytes(res).await;

            if let Err(err) = session
                .update(&hash, &response_bytes, Some(config.expire_after_seconds))
                .await
            {
                tracing::error!("Failed to cache idempotent response: {:?}", err);
                // Continue without caching
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
