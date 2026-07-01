//! Shared HTTP response helpers.
//!
//! `json_response`, `error_response`, and `into_error` are copy-pasted
//! across `onboard::api`, `deployments::api`, and `demo::http_ingress`.
//! This module is the consolidation target; new HTTP modules should import
//! from here. The pre-existing duplicates in `onboard::api` and
//! `demo::http_ingress` are left in place for a separate cleanup pass.

use http_body_util::Full;
use hyper::{Response, StatusCode, body::Bytes, header::CONTENT_TYPE};
use serde_json::{Value, json};

pub type HttpResponse = Response<Full<Bytes>>;
pub type HttpError = Box<HttpResponse>;
pub type HttpResult<T = HttpResponse> = Result<T, HttpError>;

pub fn into_error(response: HttpResponse) -> HttpError {
    Box::new(response)
}

pub fn json_response(status: StatusCode, value: Value) -> HttpResponse {
    let body = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "application/json")
        .body(Full::from(Bytes::from(body)))
        .unwrap_or_else(|err| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Full::from(Bytes::from(format!(
                    "failed to build response: {err}"
                ))))
                .unwrap()
        })
}

pub fn error_response(status: StatusCode, message: impl Into<String>) -> HttpResponse {
    json_response(
        status,
        json!({
            "success": false,
            "message": message.into(),
        }),
    )
}
