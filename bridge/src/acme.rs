//! Getting a certificate from an ACME CA (Let's Encrypt) with the DNS-01 challenge, whose TXT value
//! is published through the name service (ADR 0017). The private key is made here and stays here
//! (docs/spec/lan-cert.md L1); the CA gets a CSR, the name service gets a 43-character value.

use instant_acme::{Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder, OrderStatus, RetryPolicy};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct Issued {
    pub chain_pem: String,
    pub key_pem: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum OrderError {
    /// The CA says "not now": try again at this time (unix seconds), when it said when.
    RateLimited(Option<i64>),
    /// The name service does not know our ID: register again.
    RegistrationLost,
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum PublishError {
    Lost,
    /// The name service's weekly allowance of new orders is used up; a place is free at this time
    /// (unix seconds), when it said so.
    Quota(Option<i64>),
    Failed(String),
}

/// Where the DNS-01 TXT value goes: the name service, over HTTPS (or its own store, for the name
/// service itself).
pub trait Publisher: Send + Sync + 'static {
    fn set<'a>(&'a self, value: &'a str) -> BoxFut<'a, Result<(), PublishError>>;
    fn clear<'a>(&'a self, value: &'a str) -> BoxFut<'a, Result<(), PublishError>>;
}

/// Makes one order for one name.
pub trait Orderer: Send + Sync + 'static {
    fn order<'a>(&'a self, domain: &'a str, publisher: &'a dyn Publisher) -> BoxFut<'a, Result<Issued, OrderError>>;
}

/// Reads the time a rate-limit message names: "... retry after 2026-11-22 03:04:05 UTC ...".
pub fn retry_after(detail: &str) -> Option<i64> {
    let rest = &detail[detail.find("retry after ")? + "retry after ".len()..];
    let t = rest.get(..19)?;
    let b = t.as_bytes();
    if b[4] != b'-' || b[7] != b'-' || b[10] != b' ' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, s) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(crate::util::days_from_civil(y, mo as u32, d as u32) * 86_400 + h * 3600 + mi * 60 + s)
}

/// What a certificate and key are, once checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checked {
    pub not_before: i64,
    pub not_after: i64,
}

/// Checks a certificate before it is used (interfaces.md I33, I34): a chain, a first certificate
/// that names `domain`, a validity that includes `now`, and a key that belongs to it.
pub fn check(chain_pem: &str, key_pem: &str, domain: &str, now: i64) -> Result<Checked, String> {
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(chain_pem.as_bytes()).collect::<Result<_, _>>().map_err(|e| format!("certificate chain: {e}"))?;
    let leaf = chain.first().ok_or("certificate chain: empty")?;
    let (_, cert) = x509_parser::parse_x509_certificate(leaf).map_err(|e| format!("certificate: {e}"))?;
    let names_it = cert
        .subject_alternative_name()
        .map_err(|e| format!("certificate names: {e}"))?
        .map(|san| san.value.general_names.iter().any(|g| matches!(g, x509_parser::extensions::GeneralName::DNSName(n) if n.eq_ignore_ascii_case(domain))))
        .unwrap_or(false);
    if !names_it {
        return Err(format!("certificate does not name {domain}"));
    }
    let (not_before, not_after) = (cert.validity().not_before.timestamp(), cert.validity().not_after.timestamp());
    if now < not_before || now >= not_after {
        return Err(format!("certificate is not valid now (from {not_before} to {not_after}, now {now})"));
    }
    let key = rcgen::KeyPair::from_pem(key_pem).map_err(|e| format!("private key: {e}"))?;
    if rcgen::PublicKeyData::subject_public_key_info(&key) != cert.public_key().raw {
        return Err("the private key does not belong to the certificate".into());
    }
    Ok(Checked { not_before, not_after })
}

/// The certificate and key kept on disk (0600).
pub struct CertFiles {
    dir: PathBuf,
}

impl CertFiles {
    pub fn new(dir: &Path, prefix: &str) -> CertFiles {
        let _ = prefix;
        CertFiles { dir: dir.to_path_buf() }
    }

    /// The key and the chain are ONE file (key first): two files could be left as a new key next to
    /// an old chain by a crash between the writes, and the still valid old certificate would be lost.
    fn file(&self) -> PathBuf {
        self.dir.join("cert.pem")
    }

    /// Writes the pair, then reads it back and checks it; only a checked pair counts as saved (L7).
    pub fn save(&self, chain_pem: &str, key_pem: &str, domain: &str, now: i64) -> Result<Checked, String> {
        check(chain_pem, key_pem, domain, now)?; // nothing is written for a pair that is not good
        if key_pem.contains(CERT_MARK) {
            return Err("the key text holds a certificate".into());
        }
        crate::util::create_private_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        let both = format!("{key_pem}{chain_pem}");
        crate::util::write_private_atomic(&self.file(), both.as_bytes()).map_err(|e| format!("{}: {e}", self.file().display()))?;
        self.load(domain, now).map(|(_, _, c)| c).ok_or_else(|| "the saved certificate did not read back as valid".to_string())
    }

    /// The saved certificate, if there is one that is valid now for `domain`.
    pub fn load(&self, domain: &str, now: i64) -> Option<(String, String, Checked)> {
        let both = std::fs::read_to_string(self.file()).ok()?;
        let at = both.find(CERT_MARK)?;
        let (key, chain) = both.split_at(at);
        check(chain, key, domain, now).ok().map(|c| (chain.to_string(), key.to_string(), c))
    }
}

const CERT_MARK: &str = "-----BEGIN CERTIFICATE-----";

/// Let's Encrypt (or any ACME CA at `directory`), DNS-01.
pub struct Acme {
    dir: PathBuf,
    directory: String,
    email: String,
    /// How long to wait for the CA to check the challenge and issue.
    patience: Duration,
}

impl Acme {
    pub fn new(dir: &Path, directory: &str, email: &str) -> Acme {
        Acme { dir: dir.to_path_buf(), directory: directory.to_string(), email: email.to_string(), patience: Duration::from_secs(120) }
    }

    pub fn production() -> &'static str {
        LetsEncrypt::Production.url()
    }

    pub fn staging() -> &'static str {
        LetsEncrypt::Staging.url()
    }

    fn account_file(&self) -> PathBuf {
        // one account per CA: a staging account is not a production account
        let tag = hex::encode(&<sha2::Sha256 as sha2::Digest>::digest(self.directory.as_bytes())[..4]);
        self.dir.join(format!("acme-account-{tag}.json"))
    }

    async fn account(&self) -> Result<Account, OrderError> {
        let fail = |what: &str, e: &dyn std::fmt::Display| OrderError::Failed(format!("{what}: {e}"));
        let file = self.account_file();
        if let Ok(bytes) = std::fs::read(&file) {
            let creds: AccountCredentials = serde_json::from_slice(&bytes).map_err(|e| fail("account file", &e))?;
            let builder = Account::builder().map_err(|e| fail("ACME client", &e))?;
            return builder.from_credentials(creds).await.map_err(|e| classify(e));
        }
        let contact = if self.email.is_empty() { vec![] } else { vec![format!("mailto:{}", self.email)] };
        let contact_refs: Vec<&str> = contact.iter().map(String::as_str).collect();
        let builder = Account::builder().map_err(|e| fail("ACME client", &e))?;
        let (account, creds) = builder.create(&NewAccount { contact: &contact_refs, terms_of_service_agreed: true, only_return_existing: false }, self.directory.clone(), None).await.map_err(|e| classify(e))?;
        let json = serde_json::to_vec(&creds).map_err(|e| fail("account file", &e))?;
        crate::util::create_private_dir(&self.dir).map_err(|e| fail("account file", &e))?;
        crate::util::write_private_atomic(&file, &json).map_err(|e| fail("account file", &e))?;
        Ok(account)
    }

    async fn run(&self, domain: &str, publisher: &dyn Publisher, published: &mut Vec<String>) -> Result<Issued, OrderError> {
        let account = self.account().await?;
        let identifiers = [Identifier::Dns(domain.to_string())];
        let mut order = account.new_order(&NewOrder::new(&identifiers)).await.map_err(classify)?;
        let mut authorizations = order.authorizations();
        while let Some(result) = authorizations.next().await {
            let mut authz = result.map_err(classify)?;
            match authz.status {
                AuthorizationStatus::Pending => {}
                AuthorizationStatus::Valid => continue,
                other => return Err(OrderError::Failed(format!("authorization is {other:?}"))),
            }
            let mut challenge = authz.challenge(ChallengeType::Dns01).ok_or_else(|| OrderError::Failed("the CA offers no DNS-01 challenge".into()))?;
            let value = challenge.key_authorization().dns_value();
            publisher.set(&value).await.map_err(|e| match e {
                PublishError::Lost => OrderError::RegistrationLost,
                PublishError::Quota(at) => OrderError::RateLimited(at),
                PublishError::Failed(m) => OrderError::Failed(format!("publishing the challenge: {m}")),
            })?;
            published.push(value);
            challenge.set_ready().await.map_err(classify)?;
        }
        let policy = RetryPolicy::new().timeout(self.patience);
        let status = order.poll_ready(&policy).await.map_err(classify)?;
        if status != OrderStatus::Ready {
            return Err(OrderError::Failed(format!("the order ended as {status:?}")));
        }
        let key_pem = order.finalize().await.map_err(classify)?;
        let chain_pem = order.poll_certificate(&policy).await.map_err(classify)?;
        Ok(Issued { chain_pem, key_pem })
    }
}

fn classify(e: instant_acme::Error) -> OrderError {
    if let instant_acme::Error::Api(p) = &e {
        if p.r#type.as_deref() == Some("urn:ietf:params:acme:error:rateLimited") {
            return OrderError::RateLimited(p.detail.as_deref().and_then(retry_after));
        }
    }
    OrderError::Failed(e.to_string())
}

impl Orderer for Acme {
    fn order<'a>(&'a self, domain: &'a str, publisher: &'a dyn Publisher) -> BoxFut<'a, Result<Issued, OrderError>> {
        Box::pin(async move {
            let mut published = Vec::new();
            let result = self.run(domain, publisher, &mut published).await;
            // the TXT values are removed whether it worked or not (L5); the name service also expires them
            for v in published {
                let _ = publisher.clear(&v).await;
            }
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert_for(names: &[&str], not_before: i64, not_after: i64) -> (String, String) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut p = rcgen::CertificateParams::new(names.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap();
        let t = |s: i64| time::OffsetDateTime::from_unix_timestamp(s).unwrap();
        p.not_before = t(not_before);
        p.not_after = t(not_after);
        (p.self_signed(&key).unwrap().pem(), key.serialize_pem())
    }

    #[test]
    fn the_time_in_a_rate_limit_message_is_read() {
        let msg = "too many certificates (50) already issued for \"spellink.uk\" in the last 168h0m0s, retry after 2026-11-22 03:04:05 UTC: see https://letsencrypt.org/docs/rate-limits/";
        let t = retry_after(msg).unwrap();
        assert_eq!(t, crate::util::days_from_civil(2026, 11, 22) * 86_400 + 3 * 3600 + 4 * 60 + 5);
        for bad in ["", "retry after soon", "retry after 2026-13-22 03:04:05", "retry after 2026-11-22 25:04:05", "retry after 2026/11/22 03:04:05 UTC", "retry after 2026-11-2"] {
            assert_eq!(retry_after(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_certificate_is_used_only_if_it_names_the_domain_is_valid_now_and_has_its_key() {
        let now = 1_800_000_000;
        let (chain, key) = cert_for(&["a.lan.test"], now - 100, now + 100);
        assert_eq!(check(&chain, &key, "a.lan.test", now).unwrap(), Checked { not_before: now - 100, not_after: now + 100 });
        assert!(check(&chain, &key, "A.LAN.TEST", now).is_ok(), "names compare without case");
        assert!(check(&chain, &key, "b.lan.test", now).unwrap_err().contains("does not name"));
        assert!(check(&chain, &key, "a.lan.test", now + 100).unwrap_err().contains("not valid now"), "the last second is already over");
        assert!(check(&chain, &key, "a.lan.test", now - 101).unwrap_err().contains("not valid now"));
        let (_, other_key) = cert_for(&["a.lan.test"], now - 100, now + 100);
        assert!(check(&chain, &other_key, "a.lan.test", now).unwrap_err().contains("does not belong"));
        assert!(check("", &key, "a.lan.test", now).is_err() && check(&chain, "", "a.lan.test", now).is_err());
        let (wild, wkey) = cert_for(&["*.lan.test"], now - 100, now + 100);
        assert!(check(&wild, &wkey, "a.lan.test", now).is_err(), "a wildcard is not what we ordered");
    }

    #[test]
    fn saving_keeps_a_checked_pair_and_a_bad_one_changes_nothing() {
        let now = 1_800_000_000;
        let dir = tempfile::tempdir().unwrap();
        let files = CertFiles::new(dir.path(), "x");
        assert!(files.load("a.lan.test", now).is_none());
        let (chain, key) = cert_for(&["a.lan.test"], now - 10, now + 1000);
        files.save(&chain, &key, "a.lan.test", now).unwrap();
        let (c, k, ch) = files.load("a.lan.test", now).unwrap();
        assert_eq!((c, k, ch.not_after), (chain.clone(), key.clone(), now + 1000));
        // the files are private
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.path().join("cert.pem")).unwrap().permissions().mode() & 0o777, 0o600);
        // a certificate for another name is refused and the saved one stays
        let (c2, k2) = cert_for(&["b.lan.test"], now - 10, now + 1000);
        assert!(files.save(&c2, &k2, "a.lan.test", now).is_err());
        assert_eq!(files.load("a.lan.test", now).unwrap().0, chain);
        // when it expires it is not loaded
        assert!(files.load("a.lan.test", now + 1000).is_none());
    }
}
