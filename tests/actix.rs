#[cfg(test)]
mod tests {
  use actix_session::config::{BrowserSession, SessionLifecycle, TtlExtensionPolicy};
  use actix_web::cookie::time::Duration;
  use deadpool_redis::{Config, Runtime};
  use actix_session::storage::RedisSessionStore;
  use actix_session::SessionMiddleware;
  use actix_web::cookie::{Key, SameSite};
  use actix_web::dev::{Service, ServiceResponse};
  use actix_web::test::TestRequest;
  use actix_web::web::get;
  use actix_web::{body, test, App, Error};
  use actix_http::{Request};
  use actix_idempotent::{IdempotentFactory, IdempotentOptions};
  use bytes::Bytes;
  use std::sync::atomic::{AtomicU64, Ordering};

  static COUNTER: AtomicU64 = AtomicU64::new(0);

  async fn increment_counter() -> String {
    let count = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("Response #{}", count)
  }

  async fn create_test_app() -> impl Service<Request, Response = ServiceResponse, Error = Error> {
    let conn_string = format!("redis://:{}@{}:{}", "password", "127.0.0.1", "6379");
    let config = Config::from_url(conn_string);
    let pool = config.create_pool(Some(Runtime::Tokio1)).unwrap();
    let redis_store = RedisSessionStore::new_pooled(pool).await.unwrap();

    let idempotent_factory = IdempotentFactory::new(IdempotentOptions::default().expire_after(3));
    let secret_key = Key::generate();
    let app = test::init_service(
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
    ).await;

    app
  }

  #[tokio::test]
  async fn test_idempotency() {
    let mut app = create_test_app().await;

    let response1 = TestRequest::get()
      .uri("/test")
      .set_payload(Bytes::new())
      .send_request(&mut app).await;
    assert!(response1.status().is_success(), "Something went wrong");

    for h in response1.headers() {
      println!("{} => {:?}", h.0, h.1)
    }

    // First request should increment counter
    // Extract the session cookie from the first response
    let session_cookie = response1
      .headers()
      .get_all("set-cookie")
      .find(|&cookie| cookie.to_str().unwrap().starts_with("session="))
      .cloned()
      .expect("Session cookie not found");

    let body1 = body::to_bytes(response1.into_body()).await.unwrap();
    assert_eq!(&body1[..], b"Response #0");

    // TODO: cookie value parsing expects keyvalue pairs of `K=V` format. However security is
    // not following this pattern. Maybe because we read it from the response while a client e.g. browser
    // will not even include this part in the cookie??
    let session_cookie = session_cookie.to_str().unwrap().replace("Secure;", "");

    // Second request should return cached response - include the session cookie
    let response2 = TestRequest::get()
      .uri("/test")
      .insert_header(("cookie", session_cookie.clone()))
      .set_payload(Bytes::new())
      .send_request(&mut app).await;

    let body2 = body::to_bytes(response2.into_body()).await.unwrap();
    assert_eq!(&body2[..], b"Response #0");

    // Wait for cache to expire (ttl is set to 2 secs above)
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Third request should increment counter again - still include the session cookie
    let response3 = TestRequest::get()
      .uri("/test")
      .insert_header(("cookie", session_cookie))
      .set_payload(Bytes::new())
      .send_request(&mut app).await;

    let body3 = body::to_bytes(response3.into_body()).await.unwrap();
    assert_eq!(&body3[..], b"Response #1");
  }
}
