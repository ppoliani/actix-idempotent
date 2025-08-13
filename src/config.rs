use std::collections::HashSet;

use actix_web::http::header::{HeaderMap, HeaderName, HeaderValue};

/// Configuration options for the idempotency layer.
///
/// Configure:
/// - How long responses should be cached
/// - Which headers should be ignored when calculating the request hash
/// - Whether to ignore all headers entirely
///
/// # Example
/// ```rust
/// use axum_idempotent::IdempotentOptions;
/// use axum::http::HeaderName;
///
/// let options_1 = IdempotentOptions::default()
///     .expire_after(60) // Cache for 60 seconds
///     .ignore_header(HeaderName::from_static("x-request-id"))
///     .ignore_all_headers();
///
/// let options_2 = IdempotentOptions::new(60);
/// ```
#[derive(Clone, Debug)]
pub struct IdempotentOptions {
  pub(crate) expire_after_seconds: i64,
  pub(crate) ignored_headers: HashSet<HeaderName>,
  pub(crate) ignored_header_values: HeaderMap,
  pub(crate) ignore_all_headers: bool,
}

impl IdempotentOptions {
  pub fn new(expire_after_seconds: i64) -> Self {
    Self {
      expire_after_seconds,
      ignored_headers: HashSet::new(),
      ignored_header_values: HeaderMap::new(),
      ignore_all_headers: false,
    }
  }

  /// Sets the expiration time in seconds for cached responses.
  pub fn expire_after(mut self, seconds: i64) -> Self {
    self.expire_after_seconds = seconds;
    self
  }

  /// Adds a header to the list of headers that should be ignored when calculating the request hash.
  pub fn ignore_header(mut self, name: HeaderName) -> Self {
    self.ignored_headers.insert(name);
    self
  }

  /// Adds a header with a specific value to be ignored when calculating the request hash.
  ///
  /// If the header exists with a different value, it will still be included in the hash.
  pub fn ignore_header_with_value(mut self, name: HeaderName, value: HeaderValue) -> Self {
    self.ignored_header_values.insert(name, value);
    self
  }

  /// Configures the layer to ignore all headers when calculating the request hash.
  ///
  /// When enabled, only the method, path, and body will be used to determine idempotency.
  pub fn ignore_all_headers(mut self) -> Self {
    self.ignore_all_headers = true;
    self
  }
}

impl Default for IdempotentOptions {
  fn default() -> Self {
    let mut options = Self {
      expire_after_seconds: 300, // 5 mins default
      ignored_headers: HashSet::new(),
      ignored_header_values: HeaderMap::new(),
      ignore_all_headers: false,
    };

    let default_ignored = [
      "user-agent",
      "accept",
      "accept-encoding",
      "accept-language",
      "cache-control",
      "connection",
      "cookie",
      "host",
      "pragma",
      "referer",
      "sec-fetch-dest",
      "sec-fetch-mode",
      "sec-fetch-site",
      "sec-ch-ua",
      "sec-ch-ua-mobile",
      "sec-ch-ua-platform",
    ];

    for header in default_ignored {
      options
        .ignored_headers
        .insert(HeaderName::from_static(header));
    }

    options
  }
}
