use crate::{hash_request, IdempotentStore};
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::error::Error;
use std::str::FromStr;
use std::sync::Arc;

pub async fn check_idempotency_middleware(
    store: Arc<impl IdempotentStore>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // We have to take the request apart
    let (req, hash) = hash_request(req).await;

    let response_bytes: Option<Vec<u8>> = store
        .get(hash)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;

    if let Some(bytes) = response_bytes {
        let response =
            bytes_to_response(bytes).map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(response);
    }

    Ok(next.run(req).await)
}

/// Deserialize bytes back into a `axum::response::Response`.
fn bytes_to_response(bytes: Vec<u8>) -> Result<Response, Box<dyn Error + Send + Sync>> {
    // Split the bytes into status code, headers, and body
    let status_code_bytes = &bytes[0..2];
    let status_code = u16::from_be_bytes([status_code_bytes[0], status_code_bytes[1]]);
    let status_code = StatusCode::from_u16(status_code)?;

    // End of headers (double CRLF: \r\n\r\n)
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("Invalid header format: missing double CRLF")?;

    let header_bytes = &bytes[2..header_end];
    let headers = parse_headers(header_bytes)?;

    let body_bytes = &bytes[header_end + 4..];
    let body = Body::from(body_bytes.to_vec());

    let mut response = Response::new(body);
    *response.status_mut() = status_code;
    *response.headers_mut() = headers;

    Ok(response)
}

/// Parse headers from bytes.
fn parse_headers(header_bytes: &[u8]) -> Result<HeaderMap, Box<dyn Error + Send + Sync>> {
    let mut headers = HeaderMap::new();
    let header_str = std::str::from_utf8(header_bytes)?;

    for line in header_str.split("\r\n") {
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.splitn(2, ": ").collect();
        if parts.len() != 2 {
            return Err("Invalid header format".into());
        }

        let name = parts[0];
        let value = parts[1];
        headers.insert(HeaderName::from_str(name)?, value.parse()?);
    }

    Ok(headers)
}
