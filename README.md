# axum-idempotent

Middleware for handling idempotent requests in Axum web applications.

## Features

- Request-based idempotency without client tokens
- Configurable storage backends (Redis included)
- Automatic request hashing based on:
    - Request body
    - URI path
    - HTTP method
    - User ID (optional)

## Usage

```rust
use axum_idempotent::{IdempotentLayer, RedisStore};

// Initialize Redis store
let store = RedisStore::new(redis_pool);

// Add middleware
let app = Router::new()
    .route("/api/orders", post(create_order))
    .layer(IdempotentLayer::new(
        Arc::new(store),
        Duration::from_secs(3600)
    ));

// Optional: Add user ID for per-user idempotency
app.layer(middleware::from_fn(|req: Request, next: Next| async {
    req.extensions_mut().insert(IdempotentUserId(user_id));
    next.run(req).await
}));
```

## Custom Storage

Implement `IdempotentStore` trait for custom storage:

```rust
#[async_trait]
pub trait IdempotentStore: Clone + Send + Sync + 'static {
    async fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>, Error>;
    async fn set(&self, key: Vec<u8>, body: Vec<u8>, expire: Duration) -> Result<bool, Error>;
}
```

## License

MIT License
