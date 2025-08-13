//! Middleware for handling idempotent requests in actix-web applications.
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
//! - Integration with session-based storage (via `actix-session`)
//!
//! # Example
//!
//! ```no_run
//! use actix_session::config::{BrowserSession, SessionLifecycle, TtlExtensionPolicy};
//! use actix_web::cookie::time::Duration;
//! use deadpool_redis::{Config, Runtime};
//! use actix_session::storage::RedisSessionStore;
//! use actix_session::SessionMiddleware;
//! use actix_web::cookie::{Key, SameSite};
//! use actix_web::dev::{Service, ServiceResponse};
//! use actix_web::web::get;
//! use actix_web::App;
//! use actix_web::HttpServer;
//! use actix_idempotent::{IdempotentFactory, IdempotentOptions};
//! use std::sync::atomic::{AtomicU64, Ordering};
//! use std::io::Result;
//! 
//! static COUNTER: AtomicU64 = AtomicU64::new(0);
//! 
//! async fn increment_counter() -> String {
//!   let count = COUNTER.fetch_add(1, Ordering::SeqCst);
//!   format!("Response #{}", count)
//! }
//! 
//! #[actix_web::main]
//! async fn main() -> Result<()> {
//!   let conn_string = format!("redis://:{}@{}:{}", "password", "127.0.0.1", "6379");
//!   let config = Config::from_url(conn_string);
//!   let pool = config.create_pool(Some(Runtime::Tokio1)).unwrap();
//!   let redis_store = RedisSessionStore::new_pooled(pool).await.unwrap();
//! 
//!   let secret_key = Key::generate();
//! 
//!   HttpServer::new(move || {
//!     let idempotent_factory = IdempotentFactory::new(IdempotentOptions::default());
//! 
//!     App::new()
//!       // Add session management to your application using Redis for session state storage
//!       .wrap(
//!         SessionMiddleware::builder(redis_store.clone(), secret_key.clone())
//!           .cookie_name("session".to_string())
//!           .session_lifecycle(SessionLifecycle::BrowserSession(
//!             BrowserSession::default()
//!             .state_ttl_extension_policy(TtlExtensionPolicy::OnEveryRequest)
//!             .state_ttl(Duration::seconds(2))
//!           ))
//!           // allow the cookie to be accessed from javascript
//!           .cookie_http_only(false)
//!           // allow the cookie only from the current domain
//!           .cookie_same_site(SameSite::Strict)
//!           .build(),
//!       )
//!       .route("/test", get().to(increment_counter).wrap(idempotent_factory))
//!     })
//!     .bind("0.0.0.0:4000")?
//!     .run()
//!     .await
//! }
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
  dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform}, Error, HttpRequest
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

  fn call(&self, mut req: ServiceRequest) -> Self::Future {
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

      match check_cached_response(&hash, &session, req.request().clone()).await {
        Ok(Some(res)) => return Ok(res),
        Ok(None) => {}  // No cached response, continue
        Err(err) => {
          tracing::error!("Failed to check idempotent cached response: {:?}", err);
          // Continue without cache
        }
      }

      let res = srv.call(req).await?;
      let (res, response_bytes) = response_to_bytes(res).await?;

      if let Err(err) = session.insert(&hash, &response_bytes){
        tracing::error!("Failed to cache idempotent response: {:?}", err);
        // Continue without caching
      }

      Ok(res)
    })
  }
}

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

impl<S> Transform<S, ServiceRequest> for IdempotentFactory
where
  S: Service<ServiceRequest, Response = ServiceResponse, Error = Error> + 'static,
  S::Future: 'static,
{
  type Response = ServiceResponse;
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

async fn check_cached_response(
  hash: impl AsRef<str>,
  session: &Session,
  req: HttpRequest,
) -> Result<Option<ServiceResponse>, Box<dyn StdError + Send + Sync>> {
  let response_bytes = session.get::<Vec<u8>>(hash.as_ref())?;

  let res = if let Some(bytes) = response_bytes {
    let response = bytes_to_response(bytes)?;
    Some(ServiceResponse::new(req, response))
  } else {
    None
  };


  Ok(res)
}
