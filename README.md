# axum-idempotent

[![Documentation](https://docs.rs/axum-idempotent/badge.svg)](https://docs.rs/axum-idempotent)
[![Crates.io](https://img.shields.io/crates/v/axum-idempotent.svg)](https://crates.io/crates/axum-idempotent)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.75.0%2B-blue.svg?maxAge=3600)](
github.com/jimmielovell/ruts)

Middleware for handling idempotent requests in axum web applications.

This crate provides middleware that ensures idempotency of HTTP requests by caching responses
in a session store. When an identical request is made within the configured time window,
the cached response is returned instead of executing the request again.

## Features

- Request deduplication based on method, path, headers, and body
- Configurable response caching duration
- Header filtering options to exclude specific headers from idempotency checks
- Integration with session-based storage (via [ruts](https://crates.io/crates/ruts))

## Dependencies

This middleware requires the `SessionLayer` from the [ruts](https://crates.io/crates/ruts) crate. The layers must be added in the correct order:
1. `SessionLayer`
2. `IdempotentLayer`

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
axum-idempotent = "0.1.1"
```

## Example

```rust
use std::sync::Arc;
use axum::{Router, routing::post};
use fred::clients::Client;
use fred::interfaces::ClientLike;
use ruts::{CookieOptions, SessionLayer};
use axum_idempotent::{IdempotentLayer, IdempotentOptions};
use ruts::store::redis::RedisStore;
use tower_cookies::CookieManagerLayer;

#[tokio::main]
async fn main() {
    let client = Client::default();
    client.init().await.unwrap();
    let store = Arc::new(RedisStore::new(Arc::new(client)));

    // Configure the idempotency layer
    let idempotent_options = IdempotentOptions::default()
        .expire_after(60)  // Cache responses for 60 seconds
        .ignore_header("x-request-id".parse().unwrap());

    // Create the router with idempotency middleware
    let app = Router::new()
        .route("/payments", post(process_payment))
        .layer(IdempotentLayer::<RedisStore<Client>>::new(idempotent_options))
        .layer(SessionLayer::new(store)
            .with_cookie_options(CookieOptions::build()
                .name("session")
                .max_age(3600)
                .path("/")))
        .layer(CookieManagerLayer::new());
}

async fn process_payment() -> &'static str {
    "Payment processed"
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
