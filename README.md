# actix-idempotent

[![Documentation](https://docs.rs/actix-idempotent/badge.svg)](https://docs.rs/actix-idempotent)
[![Crates.io](https://img.shields.io/crates/v/actix-idempotent.svg)](https://crates.io/crates/actix-idempotent)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.75.0%2B-blue.svg?maxAge=3600)](https://github.com/jimmielovell/actix-idempotent)

Middleware for handling idempotent requests in actix-web applications.

This crate provides middleware that ensures idempotency of HTTP requests by caching responses
in a session store. When an identical request is made within the configured time window,
the cached response is returned instead of executing the request again.

## Features

- Request deduplication based on method, path, headers, and body
- Configurable response caching duration
- Header filtering options to exclude specific headers from idempotency checks
- Integration with session-based storage (via [actix-session](https://docs.rs/actix-session/latest/actix_session/))

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
actix-idempotent = "0.1.0"
```

## Example

```rust
use actix_session::config::{BrowserSession, SessionLifecycle, TtlExtensionPolicy};
use actix_web::cookie::time::Duration;
use deadpool_redis::{Config, Runtime};
use actix_session::storage::RedisSessionStore;
use actix_session::SessionMiddleware;
use actix_web::cookie::{Key, SameSite};
use actix_web::dev::{Service, ServiceResponse};
use actix_web::web::get;
use actix_web::App;
use actix_web::HttpServer;
use actix_idempotent::{IdempotentFactory, IdempotentOptions};
use std::sync::atomic::{AtomicU64, Ordering};
use std::io::Result;

static COUNTER: AtomicU64 = AtomicU64::new(0);

async fn increment_counter() -> String {
  let count = COUNTER.fetch_add(1, Ordering::SeqCst);
  format!("Response #{}", count)
}

#[actix_web::main]
async fn main() -> Result<()> {
  let conn_string = format!("redis://:{}@{}:{}", "password", "127.0.0.1", "6379");
  let config = Config::from_url(conn_string);
  let pool = config.create_pool(Some(Runtime::Tokio1)).unwrap();
  let redis_store = RedisSessionStore::new_pooled(pool).await.unwrap();

  let secret_key = Key::generate();

  HttpServer::new(move || {
    let idempotent_factory = IdempotentFactory::new(IdempotentOptions::default());

    App::new()
      // Add session management to your application using Redis for session state storage
      .wrap(
        SessionMiddleware::builder(redis_store.clone(), secret_key.clone())
          .cookie_name("session".to_string())
          .session_lifecycle(SessionLifecycle::BrowserSession(
            BrowserSession::default()
            .state_ttl_extension_policy(TtlExtensionPolicy::OnEveryRequest)
            .state_ttl(Duration::seconds(2))
          ))
          // allow the cookie to be accessed from javascript
          .cookie_http_only(false)
          // allow the cookie only from the current domain
          .cookie_same_site(SameSite::Strict)
          .build(),
      )
      .route("/test", get().to(increment_counter).wrap(idempotent_factory))
    })
    .bind("0.0.0.0:4000")?
    .run()
    .await
}
```

## How it Works

1. When a request is received, a hash is generated from the request method, path, headers (configurable), and body.
2. If an identical request (same hash) is found in the session store and hasn't expired:
    - The cached response is returned
    - The original handler is not called
3. If no cached response is found:
    - The request is processed normally
    - The response is cached in the session store
    - The response is returned to the client

This ensures that retrying the same request (e.g., due to network issues or client retries)
won't result in the operation being performed multiple times.

## License

This project is licensed under the MIT License.
