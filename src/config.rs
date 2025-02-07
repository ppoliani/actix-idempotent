use axum::http::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashSet;

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

    pub fn expire_after(mut self, seconds: i64) -> Self {
        self.expire_after_seconds = seconds;
        self
    }

    pub fn ignore_header(mut self, name: HeaderName) -> Self {
        self.ignored_headers.insert(name);
        self
    }

    pub fn ignore_header_with_value(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.ignored_header_values.insert(name, value);
        self
    }

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
            ignore_all_headers: false
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
            options.ignored_headers.insert(
                HeaderName::from_static(header)
            );
        }

        options
    }
}
