//! Talking to the name service over HTTPS (interfaces.md I32), and the same two jobs done
//! directly against the store when this process is the name service.

use super::{Clock, Registrar};
use crate::acme::{BoxFut, PublishError, Publisher};
use crate::names::store::{ApiError, Names};
use crate::namesproto::Identity;
use std::sync::Arc;
use std::time::Duration;

/// The base address of a name service: https, a host, no credentials, query or fragment. http is
/// only for the machine itself (tests, a name service behind a local proxy).
pub fn names_url(s: &str) -> Result<String, String> {
    let u = url::Url::parse(s).map_err(|e| format!("{s:?}: {e}"))?;
    let loopback = matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match u.scheme() {
        "https" => {}
        "http" if loopback => {}
        other => return Err(format!("{s:?}: the scheme must be https, not {other}")),
    }
    if u.host_str().is_none() || !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() || !(u.path() == "" || u.path() == "/") {
        return Err(format!("{s:?}: must be only a scheme, host and port"));
    }
    Ok(u.as_str().trim_end_matches('/').to_string())
}

pub struct NamesClient {
    base: String,
    identity: Arc<Identity>,
    clock: Clock,
    agent: ureq::Agent,
}

impl NamesClient {
    pub fn new(base: &str, identity: Arc<Identity>, clock: Clock) -> Result<NamesClient, String> {
        let base = names_url(base)?;
        crate::util::install_crypto();
        let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build();
        Ok(NamesClient { base, identity, clock, agent: ureq::Agent::new_with_config(config) })
    }

    /// POSTs a JSON body; Ok(()) on 200, otherwise the status and the name the service gave.
    async fn post(&self, path: &'static str, body: Vec<u8>) -> Result<(), (u16, String, Option<i64>)> {
        let (agent, url) = (self.agent.clone(), format!("{}{path}", self.base));
        // ureq blocks: it runs where blocking is allowed
        let result = tokio::task::spawn_blocking(move || {
            let mut resp = agent.post(&url).content_type("application/json").send(&body[..]).map_err(|e| (0u16, e.to_string()))?;
            let status = resp.status().as_u16();
            // the answer is small: a name like "unknown_id"
            let text = resp.body_mut().with_config().limit(4096).read_to_string().unwrap_or_default();
            Ok::<_, (u16, String)>((status, text))
        })
        .await
        .map_err(|e| (0, e.to_string(), None))?;
        let (status, text) = result.map_err(|(s, n)| (s, n, None))?;
        if status == 200 {
            return Ok(());
        }
        let parsed = serde_json::from_str::<serde_json::Value>(&text).ok();
        let name = parsed.as_ref().and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from)).unwrap_or_default();
        // when a place is free again (only a plausible time is used; anything else is ignored)
        let retry = parsed.as_ref().and_then(|v| v.get("retryAfter").and_then(|t| t.as_i64())).filter(|t| (0..=i64::MAX / 2).contains(t));
        Err((status, name, retry))
    }

    fn describe(status: u16, name: &str) -> String {
        match status {
            0 => format!("the name service could not be reached: {name}"),
            s => format!("the name service answered {s} {name}"),
        }
    }
}

impl Registrar for NamesClient {
    fn register<'a>(&'a self, ip: &'a str) -> BoxFut<'a, Result<(), String>> {
        Box::pin(async move {
            let req = self.identity.register(ip, (self.clock)());
            let body = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
            self.post("/v1/register", body).await.map_err(|(s, n, _)| NamesClient::describe(s, &n))
        })
    }
}

impl Publisher for NamesClient {
    fn set<'a>(&'a self, value: &'a str) -> BoxFut<'a, Result<(), PublishError>> {
        Box::pin(self.acme("set", value))
    }
    fn clear<'a>(&'a self, value: &'a str) -> BoxFut<'a, Result<(), PublishError>> {
        Box::pin(self.acme("clear", value))
    }
}

impl NamesClient {
    async fn acme(&self, op: &'static str, value: &str) -> Result<(), PublishError> {
        let req = self.identity.acme(op, value, (self.clock)());
        let body = serde_json::to_vec(&req).map_err(|e| PublishError::Failed(e.to_string()))?;
        self.post("/v1/acme", body).await.map_err(|(s, n, retry)| match (s, n.as_str()) {
            (404, _) => PublishError::Lost,
            (429, "lan_quota") => PublishError::Quota(retry),
            _ => PublishError::Failed(NamesClient::describe(s, &n)),
        })
    }
}

/// The name service writing its own challenge records: the same signed requests, handed to the
/// store directly instead of over the network.
pub struct LocalNames {
    pub names: Arc<Names>,
    pub identity: Arc<Identity>,
    pub clock: Clock,
}

impl Registrar for LocalNames {
    fn register<'a>(&'a self, _ip: &'a str) -> BoxFut<'a, Result<(), String>> {
        Box::pin(async move {
            self.names.add_local(self.identity.id(), self.identity.public_key(), (self.clock)());
            Ok(())
        })
    }
}

impl Publisher for LocalNames {
    fn set<'a>(&'a self, value: &'a str) -> BoxFut<'a, Result<(), PublishError>> {
        Box::pin(async move { self.run("set", value) })
    }
    fn clear<'a>(&'a self, value: &'a str) -> BoxFut<'a, Result<(), PublishError>> {
        Box::pin(async move { self.run("clear", value) })
    }
}

impl LocalNames {
    fn run(&self, op: &str, value: &str) -> Result<(), PublishError> {
        let now = (self.clock)();
        match self.names.acme(&self.identity.acme(op, value, now), now) {
            Ok(()) => Ok(()),
            Err(ApiError::NotFound) => Err(PublishError::Lost),
            Err(ApiError::Quota(at)) => Err(PublishError::Quota(Some(at))),
            Err(e) => Err(PublishError::Failed(e.name().to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_service_address_is_https_and_only_an_address() {
        assert_eq!(names_url("https://names.spellink.uk:8443").unwrap(), "https://names.spellink.uk:8443");
        assert_eq!(names_url("https://names.spellink.uk/").unwrap(), "https://names.spellink.uk");
        assert_eq!(names_url("http://127.0.0.1:9000").unwrap(), "http://127.0.0.1:9000");
        for bad in ["http://names.spellink.uk", "ftp://x", "https://user:pw@x.test", "https://x.test/path", "https://x.test?a=1", "https://x.test#f", "names.spellink.uk", "", "https://"] {
            assert!(names_url(bad).is_err(), "{bad}");
        }
    }
}
