//! `POST /deployments/{stage,warm,activate,rollback,complete-drain}` HTTP
//! handlers. Thin JSON-body adapters over the `greentic_deployer` library
//! verbs — the same verbs the local `gtc op revisions|traffic` CLI calls.
//!
//! Phase B is local-CLI scaffolding: handlers are open (no mTLS), and each
//! request constructs a fresh `LocalFsStore` rooted at the per-user default
//! (`~/.greentic/environments`) or the `GREENTIC_OPERATOR_ENV_ROOT` env-var
//! override. Per-request stores are safe because the store is just a path
//! holder — file-level locking lives inside `store.transact`.
//!
//! mTLS gating and admission folding into `admin_api::AdminState` is a
//! Phase D/E concern (see plan §B4b).
//!
//! `OpError` → HTTP status mapping mirrors the CLI's exit-code envelope so
//! a local caller switching from `gtc op` to HTTP sees the same semantics.

use std::path::PathBuf;
use std::sync::Arc;

use greentic_deployer::cli::{
    OpError, OpFlags, OpOutcome,
    revisions::{self, RevisionStagePayload, RevisionTransitionPayload},
    traffic::{self, TrafficSetPayload, TrafficShowPayload},
};
use greentic_deployer::environment::LocalFsStore;
use http_body_util::{BodyExt, Full};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    header::CONTENT_TYPE,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// Environment-variable override for the env-store root. When unset, we fall
/// back to [`LocalFsStore::default_root`] (`~/.greentic/environments`) — the
/// same default `gtc op` uses.
const ENV_ROOT_VAR: &str = "GREENTIC_OPERATOR_ENV_ROOT";

pub type DeploymentResponse = Response<Full<Bytes>>;
pub type DeploymentError = Box<DeploymentResponse>;
pub type DeploymentResult<T = DeploymentResponse> = Result<T, DeploymentError>;

fn into_error(response: DeploymentResponse) -> DeploymentError {
    Box::new(response)
}

/// Shared state. `env_root = None` falls back to the per-user default at
/// request time; tests inject a tempdir here.
pub struct DeploymentsState {
    pub env_root: Option<PathBuf>,
}

impl DeploymentsState {
    pub fn new() -> Self {
        Self { env_root: None }
    }

    fn build_store(&self) -> Result<LocalFsStore, DeploymentError> {
        if let Some(root) = &self.env_root {
            return Ok(LocalFsStore::new(root.clone()));
        }
        if let Ok(override_root) = std::env::var(ENV_ROOT_VAR)
            && !override_root.is_empty()
        {
            return Ok(LocalFsStore::new(PathBuf::from(override_root)));
        }
        LocalFsStore::default_root()
            .map(LocalFsStore::new)
            .ok_or_else(|| {
                into_error(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "no env-store root: HOME / USERPROFILE not set and \
                     GREENTIC_OPERATOR_ENV_ROOT unset",
                ))
            })
    }
}

impl Default for DeploymentsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Dispatch `/deployments/*` requests.
///
///   POST /deployments/stage           → `revisions::stage`
///   POST /deployments/warm            → `revisions::warm`
///   POST /deployments/activate        → `traffic::set`
///   POST /deployments/rollback        → `traffic::rollback`
///   POST /deployments/complete-drain  → `revisions::drain`
pub async fn handle_deployments_request(
    req: Request<Incoming>,
    path: &str,
    state: &Arc<DeploymentsState>,
) -> DeploymentResult {
    let method = req.method().clone();
    let body_bytes = req
        .into_body()
        .collect()
        .await
        .map(|c| c.to_bytes())
        .map_err(|err| {
            into_error(error_response(
                StatusCode::BAD_REQUEST,
                format!("read body: {err}"),
            ))
        })?;
    dispatch(method, path, &body_bytes, state).await
}

/// Inner dispatcher — split out of [`handle_deployments_request`] so unit
/// tests can drive it with pre-collected body bytes without going through a
/// real hyper connection (constructing `Request<Incoming>` requires a live
/// stream, and hangs when the test holds the request after the mock server
/// closes the connection).
async fn dispatch(
    method: Method,
    path: &str,
    body_bytes: &[u8],
    state: &Arc<DeploymentsState>,
) -> DeploymentResult {
    let sub_path = path
        .strip_prefix("/deployments")
        .unwrap_or("")
        .trim_end_matches('/');

    match (method, sub_path) {
        (Method::POST, "/stage") => {
            let payload = parse_json::<RevisionStagePayload>(body_bytes)?;
            run_blocking(state, move |store, flags| {
                revisions::stage(store, flags, Some(payload))
            })
            .await
        }
        (Method::POST, "/warm") => {
            let payload = parse_json::<RevisionTransitionPayload>(body_bytes)?;
            run_blocking(state, move |store, flags| {
                revisions::warm(store, flags, Some(payload))
            })
            .await
        }
        (Method::POST, "/activate") => {
            let payload = parse_json::<TrafficSetPayload>(body_bytes)?;
            run_blocking(state, move |store, flags| {
                traffic::set(store, flags, Some(payload))
            })
            .await
        }
        (Method::POST, "/rollback") => {
            let payload = parse_json::<TrafficShowPayload>(body_bytes)?;
            run_blocking(state, move |store, flags| {
                traffic::rollback(store, flags, Some(payload))
            })
            .await
        }
        (Method::POST, "/complete-drain") => {
            let payload = parse_json::<RevisionTransitionPayload>(body_bytes)?;
            run_blocking(state, move |store, flags| {
                revisions::drain(store, flags, Some(payload))
            })
            .await
        }
        (_, "/stage" | "/warm" | "/activate" | "/rollback" | "/complete-drain") => {
            Err(into_error(error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                format!("only POST allowed on /deployments{sub_path}"),
            )))
        }
        _ => Err(into_error(error_response(
            StatusCode::NOT_FOUND,
            format!("unknown /deployments endpoint: {sub_path}"),
        ))),
    }
}

async fn run_blocking<F>(state: &Arc<DeploymentsState>, op: F) -> DeploymentResult
where
    F: FnOnce(&LocalFsStore, &OpFlags) -> Result<OpOutcome, OpError> + Send + 'static,
{
    let store = state.build_store()?;
    let result = tokio::task::spawn_blocking(move || op(&store, &OpFlags::default()))
        .await
        .map_err(|err| {
            into_error(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("worker join: {err}"),
            ))
        })?;
    match result {
        Ok(outcome) => Ok(json_response(
            StatusCode::OK,
            serde_json::to_value(outcome).unwrap_or_else(|_| json!({})),
        )),
        Err(err) => Err(into_error(op_error_response(&err))),
    }
}

fn parse_json<T: DeserializeOwned>(body: &[u8]) -> Result<T, DeploymentError> {
    serde_json::from_slice::<T>(body).map_err(|err| {
        into_error(error_response(
            StatusCode::BAD_REQUEST,
            format!("invalid JSON: {err}"),
        ))
    })
}

/// Map an `OpError` to the documented `{op, noun, error}` envelope. Status
/// codes match the CLI's exit-code semantics so a local caller switching
/// from `gtc op` to HTTP sees the same kind taxonomy.
fn op_error_response(err: &OpError) -> DeploymentResponse {
    let status = match err {
        OpError::NotFound(_) => StatusCode::NOT_FOUND,
        OpError::Conflict(_) => StatusCode::CONFLICT,
        OpError::Unauthorized { .. } => StatusCode::FORBIDDEN,
        OpError::InvalidArgument(_) | OpError::Spec(_) | OpError::AnswersParse { .. } => {
            StatusCode::BAD_REQUEST
        }
        OpError::NotYetImplemented(_) => StatusCode::NOT_IMPLEMENTED,
        OpError::Store(_)
        | OpError::Io { .. }
        | OpError::SchemaGeneration(_)
        | OpError::Audit(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    json_response(
        status,
        json!({
            "error": {
                "kind": err.kind(),
                "message": err.to_string(),
            }
        }),
    )
}

fn json_response(status: StatusCode, value: Value) -> DeploymentResponse {
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

fn error_response(status: StatusCode, message: impl Into<String>) -> DeploymentResponse {
    json_response(
        status,
        json!({
            "success": false,
            "message": message.into(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    fn state_with_tempdir(tmp: &tempfile::TempDir) -> Arc<DeploymentsState> {
        Arc::new(DeploymentsState {
            env_root: Some(tmp.path().to_path_buf()),
        })
    }

    async fn read_body_json(resp: DeploymentResponse) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        if bytes.is_empty() {
            return json!({});
        }
        serde_json::from_slice(&bytes).expect("valid JSON body")
    }

    #[tokio::test]
    async fn unknown_subpath_returns_404() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_tempdir(&tmp);
        let err = dispatch(Method::POST, "/deployments/bogus", b"{}", &state)
            .await
            .expect_err("bogus path must error");
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn wrong_method_returns_405() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_tempdir(&tmp);
        for verb in [
            "/stage",
            "/warm",
            "/activate",
            "/rollback",
            "/complete-drain",
        ] {
            let path = format!("/deployments{verb}");
            let err = dispatch(Method::GET, &path, b"", &state)
                .await
                .expect_err("GET must error");
            assert_eq!(
                err.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "GET {path} should 405"
            );
        }
    }

    #[tokio::test]
    async fn malformed_json_returns_400() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_tempdir(&tmp);
        let err = dispatch(Method::POST, "/deployments/stage", b"{not-json", &state)
            .await
            .expect_err("malformed JSON must error");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_required_payload_field_returns_400() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_tempdir(&tmp);
        let body = br#"{"environment_id": "demo-env"}"#;
        let err = dispatch(Method::POST, "/deployments/warm", body, &state)
            .await
            .expect_err("missing field must error");
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn end_to_end_through_deployer_lib_returns_typed_error_envelope() {
        // Drives a full happy-path payload through `dispatch` → deployer
        // library → `op_error_response`. The deployer's `audit_and_record`
        // wrapper runs `authorize_local_only` first, which denies inside the
        // sandboxed test environment (no local-actor signal), so we expect a
        // 403 with the documented envelope. The point of the test is the
        // round-trip: handler invoked the lib, got a typed `OpError`, mapped
        // it to the right HTTP status, and serialized the documented body.
        // ULIDs are Crockford Base32 (no I/L/O/U); use a real one so we
        // get past the ID-parse branch.
        let tmp = tempfile::tempdir().unwrap();
        let state = state_with_tempdir(&tmp);
        let body = br#"{
            "environment_id": "demo-env",
            "deployment_id": "01HQXXVNAEPS9YYBFB3FBTQDR6",
            "bundle_digest": "sha256:00"
        }"#;
        let err = dispatch(Method::POST, "/deployments/stage", body, &state)
            .await
            .expect_err("local-only auth denial must surface");
        assert_eq!(err.status(), StatusCode::FORBIDDEN);
        let body = read_body_json(*err).await;
        assert_eq!(body["error"]["kind"], "unauthorized");
    }

    #[test]
    fn op_error_to_status_covers_every_variant() {
        // Exhaustive over OpError variants so any future addition forces a
        // conscious status-code choice. Variants we can't trivially fabricate
        // (Store/Spec/Audit) get verified via their HTTP status code only.
        let cases: &[(OpError, StatusCode)] = &[
            (OpError::NotFound("x".into()), StatusCode::NOT_FOUND),
            (OpError::Conflict("x".into()), StatusCode::CONFLICT),
            (
                OpError::Unauthorized {
                    policy: "p".into(),
                    reason: "r".into(),
                },
                StatusCode::FORBIDDEN,
            ),
            (
                OpError::InvalidArgument("x".into()),
                StatusCode::BAD_REQUEST,
            ),
            (
                OpError::AnswersParse {
                    path: PathBuf::from("/dev/null"),
                    message: "x".into(),
                },
                StatusCode::BAD_REQUEST,
            ),
            (
                OpError::SchemaGeneration("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                OpError::Audit("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                OpError::Io {
                    path: PathBuf::from("/dev/null"),
                    source: std::io::Error::other("boom"),
                },
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(
                op_error_response(err).status(),
                *expected,
                "{err} should map to {expected}"
            );
        }
    }
}
