# axum-idempotent

Middleware for handling idempotent requests in [Axum](https://docs.rs/axum/latest/axum/index.html) web applications.

## Features

- Request-based idempotency without client tokens
- Configurable storage backends (from [ruts](https://crates.io/crates/ruts))
- Automatic request hashing.
