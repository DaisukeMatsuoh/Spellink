//! The bridge's own command line and pages talk to the running bridge here (Go: server/admin.go).
//! Served under /admin/ and only to a caller that did not come through a proxy or tunnel and knows
//! the token (stored in the bridge's config directory, mode 0600).

use crate::devices::registry::PackageData;
use crate::devices::{Error, Registry};
use crate::localguard;
use crate::timestamp::Timestamp;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const MAX_ADMIN_BODY: usize = 4 << 10;
/// The AI settings hold up to 2000 characters of instructions (6 KB in Japanese) and 32 rules.
const MAX_AI_BODY: usize = 32 << 10;

/// What the bridge's own command line needs: the ledger, a way to build a device's .snplg, and a
/// bearer token.
#[derive(Clone)]
pub struct AdminDeps {
    pub devices: Registry,
    /// The bearer token of the command line (high-entropy; never logged).
    pub token: String,
    /// The .snplg for a device, built from the data the ledger gives (REPACK).
    pub package: Arc<dyn Fn(PackageData) -> Result<Vec<u8>, String> + Send + Sync>,
    /// What the app page shows about the bridge itself (none: only the device counts).
    pub status: Option<Arc<dyn Fn() -> StatusInfo + Send + Sync>>,
    /// Reading and changing the AI settings (ADR 0022); none: the app page has no such section.
    pub ai: Option<AiAdmin>,
}

/// What the AI settings of the app page are (ADR 0022: the owner's own computer is the only place
/// that changes them; this route is local and carries the admin token).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiSettings {
    pub environment: bool,
    pub instructions: String,
    pub allow: Vec<String>,
}

#[derive(Clone)]
pub struct AiAdmin {
    /// The saved settings, and whether they differ from what the running bridge uses.
    pub get: Arc<dyn Fn() -> (AiSettings, bool) + Send + Sync>,
    /// Checks and saves; an error is the reason (it reaches the owner as it is).
    pub set: Arc<dyn Fn(AiSettings) -> Result<(), String> + Send + Sync>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AiJson {
    #[serde(flatten)]
    settings: AiSettings,
    /// Saved, but the running bridge still uses the earlier ones: a restart makes them take effect.
    restart_needed: bool,
}

/// What the bridge knows about itself that the ledger does not.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusInfo {
    pub version: String,
    pub name: String,
    /// The AI the bridge asks: its name, and whether its program can be found here.
    pub provider: ProviderInfo,
    /// True when the bridge registers with a relay (devices can reach it away from home).
    pub relay: bool,
    /// The wss:// addresses a new device would be told.
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub found: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
}

#[derive(Serialize)]
struct DevicesCount {
    total: usize,
    active: usize,
    waiting: usize,
}

#[derive(Serialize)]
struct StatusJson {
    #[serde(flatten)]
    info: StatusInfo,
    devices: DevicesCount,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceJson {
    id: String,
    name: String,
    #[serde(rename = "gen")]
    generation: i64,
    status: String,
    waiting: usize,
    issued_at: Timestamp,
    #[serde(skip_serializing_if = "Timestamp::is_zero")]
    expires: Timestamp,
    #[serde(skip_serializing_if = "Timestamp::is_zero")]
    last_seen: Timestamp,
}

/// Maps the ledger's errors to what the command line shows. The message is the ledger's own
/// sentence (never a secret, never which devices are waiting for a code).
fn admin_error(e: Error) -> Response {
    let status = match &e {
        Error::UnknownDevice | Error::NoPending => StatusCode::NOT_FOUND,
        Error::BadName | Error::BadCode => StatusCode::BAD_REQUEST,
        Error::CodeRejected => StatusCode::FORBIDDEN,
        Error::NameTaken | Error::Full | Error::AlreadyRevoked | Error::Revoked | Error::Expired | Error::StillValid => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, format!("{e}\n")).into_response()
}

async fn read_json<T: for<'de> Deserialize<'de>>(req: Request) -> Result<T, Response> {
    read_json_max(req, MAX_ADMIN_BODY).await
}

async fn read_json_max<T: for<'de> Deserialize<'de>>(req: Request, max: usize) -> Result<T, Response> {
    let bytes = axum::body::to_bytes(req.into_body(), max).await.map_err(|_| (StatusCode::BAD_REQUEST, "bad request\n").into_response())?;
    serde_json::from_slice(&bytes).map_err(|_| (StatusCode::BAD_REQUEST, "bad request\n").into_response())
}

/// Lets a request through only if it is local and carries the token.
async fn guard(State(a): State<AdminDeps>, req: Request, next: Next) -> Response {
    if !localguard::local(req.headers(), &localguard::host_of(&req)) {
        return localguard::refuse();
    }
    let given = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    let given = given.strip_prefix("Bearer ").unwrap_or(given);
    let (got, want): ([u8; 32], [u8; 32]) = (Sha256::digest(given.as_bytes()).into(), Sha256::digest(a.token.as_bytes()).into());
    let same = got.iter().zip(&want).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0;
    if a.token.is_empty() || !same {
        return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
    }
    next.run(req).await
}

impl AdminDeps {
    /// The /admin/ routes (guarded).
    pub fn router(&self) -> Router {
        let a = self.clone();
        Router::new()
            .route(
                "/admin/status",
                get(|State(a): State<AdminDeps>| async move {
                    let mut info = a.status.as_ref().map(|f| f()).unwrap_or_default();
                    let mut devices = DevicesCount { total: 0, active: 0, waiting: 0 };
                    for d in a.devices.list() {
                        devices.total += 1;
                        if d.status == "active" {
                            devices.active += 1;
                        }
                        devices.waiting += d.waiting;
                    }
                    info.addresses.shrink_to_fit();
                    Json(StatusJson { info, devices })
                }),
            )
            .route(
                "/admin/devices",
                get(|State(a): State<AdminDeps>| async move {
                    let out: Vec<DeviceJson> = a
                        .devices
                        .list()
                        .into_iter()
                        .map(|d| DeviceJson { id: d.id, name: d.name, generation: d.generation, status: d.status, waiting: d.waiting, issued_at: d.issued_at, expires: d.expires, last_seen: d.last_seen })
                        .collect();
                    Json(out)
                })
                .post(|State(a): State<AdminDeps>, req: Request| async move {
                    #[derive(Deserialize)]
                    struct In {
                        #[serde(default)]
                        name: String,
                    }
                    let input: In = match read_json(req).await {
                        Ok(i) => i,
                        Err(r) => return r,
                    };
                    match a.devices.add(&input.name) {
                        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({"id": id, "name": input.name}))).into_response(),
                        Err(e) => admin_error(e),
                    }
                }),
            )
            .route("/admin/devices/{id}/revoke", post(|State(a): State<AdminDeps>, Path(id): Path<String>| async move { done(a.devices.revoke(&id)) }))
            .route("/admin/devices/{id}/reissue", post(|State(a): State<AdminDeps>, Path(id): Path<String>| async move { done(a.devices.reissue(&id)) }))
            .route("/admin/devices/{id}/forget", post(|State(a): State<AdminDeps>, Path(id): Path<String>| async move { done(a.devices.forget(&id)) }))
            .route(
                "/admin/ai",
                get(|State(a): State<AdminDeps>| async move {
                    match &a.ai {
                        None => StatusCode::NOT_FOUND.into_response(),
                        Some(ai) => {
                            let (settings, restart_needed) = (ai.get)();
                            Json(AiJson { settings, restart_needed }).into_response()
                        }
                    }
                })
                .put(|State(a): State<AdminDeps>, req: Request| async move {
                    let Some(ai) = a.ai.clone() else { return StatusCode::NOT_FOUND.into_response() };
                    let input: AiSettings = match read_json_max(req, MAX_AI_BODY).await {
                        Ok(i) => i,
                        Err(r) => return r,
                    };
                    match (ai.set)(input) {
                        Ok(()) => {
                            let (settings, restart_needed) = (ai.get)();
                            Json(AiJson { settings, restart_needed }).into_response()
                        }
                        Err(why) => (StatusCode::BAD_REQUEST, format!("{why}\n")).into_response(),
                    }
                }),
            )
            .route(
                "/admin/devices/{id}/package",
                get(|State(a): State<AdminDeps>, Path(id): Path<String>| async move {
                    let d = match a.devices.repack(&id) {
                        Ok(d) => d,
                        Err(e) => return admin_error(e),
                    };
                    let name = d.name.clone();
                    match (a.package)(d) {
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")).into_response(),
                        Ok(pkg) => {
                            let mut r = Response::new(Body::from(pkg));
                            r.headers_mut().insert("content-type", HeaderValue::from_static("application/octet-stream"));
                            // the name is the owner's own text: keep it a valid header value
                            if let Ok(v) = HeaderValue::from_str(&name) {
                                r.headers_mut().insert("x-device-name", v);
                            } else if let Ok(v) = HeaderValue::from_str(&urlencode(&name)) {
                                r.headers_mut().insert("x-device-name", v);
                            }
                            r
                        }
                    }
                }),
            )
            .route(
                "/admin/pair",
                post(|State(a): State<AdminDeps>, req: Request| async move {
                    #[derive(Deserialize)]
                    struct In {
                        #[serde(default)]
                        code: String,
                        /// The device's id when the code was read from its QR code; then only that device is considered.
                        #[serde(default)]
                        id: String,
                    }
                    let input: In = match read_json(req).await {
                        Ok(i) => i,
                        Err(r) => return r,
                    };
                    let r = if input.id.is_empty() { a.devices.confirm(&input.code) } else { a.devices.confirm_for(&input.id, &input.code) };
                    match r {
                        Ok(name) => Json(serde_json::json!({"name": name})).into_response(),
                        Err(e) => admin_error(e),
                    }
                }),
            )
            .with_state(a.clone())
            .layer(middleware::from_fn_with_state(a, guard))
    }
}

fn done(r: Result<(), Error>) -> Response {
    match r {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => admin_error(e),
    }
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
