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

use actix_web::{
  dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform}, Error
};
use std::{
  future::{ready, Ready as StdReady},
  rc::Rc, error::Error as StdError,
};
use actix_session::Session;
use futures_util::future::LocalBoxFuture;

mod utils;

mod config;
pub use crate::config::IdempotentOptions;

use crate::utils::{bytes_to_response, hash_request, response_to_bytes};

pub struct IdempotentMiddleware<S> {
  service: Rc<S>,
  config: IdempotentOptions,
}

impl<S> Service<ServiceRequest> for IdempotentMiddleware<S>
where
  S: Service<ServiceRequest, Response = ServiceResponse, Error = Error> + 'static,
{
  type Response = ServiceResponse;
  type Error = Error;
  type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

  forward_ready!(service);

  fn call(&self, req: ServiceRequest) -> Self::Future {
    let srv = self.service.clone();
    let config = self.config.clone();

    Box::pin(async move {
      let session = match req.extract::<Session>().await {
        Ok(session) => session,
        Err(err) => {
          tracing::error!("Failed to extract Session from request: {:?}", err);
          // Forward the request to the inner service without idempotency
          return srv.call(req).await;
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

      let res = srv.call(req).await?;
      let (res, response_bytes) = response_to_bytes(res).await;

      if let Err(err) = session.insert(&hash, &response_bytes){
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
pub struct IdempotentFactory {
  config: IdempotentOptions,
}

impl IdempotentFactory {
  pub const fn new(config: IdempotentOptions) -> Self {
    IdempotentFactory {
      config,
    }
  }
}

impl<S, B> Transform<S, ServiceRequest> for IdempotentFactory
where
  S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
  S::Future: 'static,
  B: 'static,
{
  type Response = ServiceResponse<B>;
  type Error = Error;
  type InitError = ();
  type Transform = IdempotentMiddleware<S>;
  type Future = StdReady<Result<Self::Transform, Self::InitError>>;

  fn new_transform(&self, service: S) -> Self::Future {
    ready(Ok(IdempotentMiddleware {
      service: Rc::new(service),
      config: self.config.clone(),
    }))
  }
}

async fn check_cached_response<B>(
  hash: impl AsRef<str>,
  session: &Session,
) -> Result<Option<ServiceResponse<B>>, Box<dyn StdError + Send + Sync>> {
  let response_bytes = session.get::<Vec<u8>>(hash.as_ref())?;

  let res = if let Some(bytes) = response_bytes {
    let response = bytes_to_response(bytes)?;

    Some(response)
  } else {
    None
  };

  Ok(res)
}
