#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::{body::Body, extract::Request, routing::get, Router};
    use axum_idempotent::{IdempotentLayer, IdempotentOptions};
    use fred::clients::Client;
    use fred::prelude::ClientLike;
    use ruts::store::redis::RedisStore;
    use ruts::{CookieOptions, SessionLayer};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;
    use tower_cookies::CookieManagerLayer;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn increment_counter() -> String {
        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("Response #{}", count)
    }

    async fn setup_redis() -> RedisStore<Client> {
        let client = Client::default();
        client.init().await.unwrap();

        RedisStore::new(Arc::new(client))
    }

    async fn create_test_app() -> Router {
        let store = Arc::new(setup_redis().await);

        // Configure session options
        let cookie_options = CookieOptions::build().name("session").max_age(10).path("/");
        let session_layer = SessionLayer::new(store.clone()).with_cookie_options(cookie_options);

        let idempotent_options = IdempotentOptions::default().expire_after(3);
        let idempotent_layer = IdempotentLayer::<RedisStore<Client>>::new(idempotent_options);

        Router::new()
            .route("/test", get(increment_counter))
            .layer(idempotent_layer)
            .layer(session_layer)
            .layer(CookieManagerLayer::new())
    }

    #[tokio::test]
    async fn test_idempotency() {
        let app = create_test_app().await;

        // First request should increment counter
        let response1 = app
            .clone()
            .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        // Extract the session cookie from the first response
        let session_cookie = response1
            .headers()
            .get_all("set-cookie")
            .iter()
            .find(|&cookie| cookie.to_str().unwrap().starts_with("session="))
            .cloned()
            .expect("Session cookie not found");

        let body1 = to_bytes(response1.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body1[..], b"Response #0");

        // Second request should return cached response - include the session cookie
        let response2 = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("cookie", &session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body2 = to_bytes(response2.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body2[..], b"Response #0");

        // Wait for cache to expire
        tokio::time::sleep(Duration::from_secs(3)).await;

        // Third request should increment counter again - still include the session cookie
        let response3 = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .header("cookie", &session_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body3 = to_bytes(response3.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body3[..], b"Response #1");
    }
}
