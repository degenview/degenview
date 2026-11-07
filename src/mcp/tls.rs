//! The certificate a server on a named network interface presents.
//!
//! Loopback needs none. A server reachable from other machines is served over TLS only, with a
//! self-signed certificate made here for the one address it listens on. Nothing vouches for a
//! self-signed certificate, so the client pins it: [`TlsIdentity::fingerprint`] is what the user
//! compares (or the client stores) the first time it connects.
//!
//! The certificate and its private key live in one PEM file in the app's data directory, written
//! owner-only on Unix. The key is not part of the credential seal: a process that can replace the
//! file can present a different certificate, but it learns no bearer token from that, and a client
//! that pinned the fingerprint refuses the new one. TLS 1.3 only; the provider is `ring`, chosen
//! explicitly so no process-wide default has to be installed.

use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{self, ServerConfig};

/// The file inside the data directory.
pub const FILE_NAME: &str = "mcp-tls.pem";

/// First line of the file, ahead of the PEM blocks (a PEM reader skips text outside them). It
/// records which address the certificate was made for, so a changed address is noticed without
/// parsing the certificate.
const ADDRESS_PREFIX: &str = "# degenview-mcp-tls address=";

/// A certificate and the key that signs for it.
#[derive(Clone)]
pub struct TlsIdentity {
    address: IpAddr,
    cert_pem: String,
    key_pem: String,
    cert_der: Vec<u8>,
}

impl std::fmt::Debug for TlsIdentity {
    // The key is never printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsIdentity")
            .field("address", &self.address)
            .field("fingerprint", &self.fingerprint())
            .finish_non_exhaustive()
    }
}

impl TlsIdentity {
    /// A new self-signed certificate valid for `address`, which is its only name.
    pub fn generate(address: IpAddr) -> Result<Self, String> {
        let key = KeyPair::generate().map_err(|error| format!("Could not make a key: {error}"))?;
        let mut params = CertificateParams::new(vec![address.to_string()])
            .map_err(|error| format!("Could not make a certificate: {error}"))?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "DegenView MCP");
        params.distinguished_name = name;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        // The client pins this certificate, so there is nothing to renew on a schedule. The start
        // is a fixed date in the past rather than the clock, so a machine with a wrong time still
        // validates it.
        params.not_before = date_time_ymd(2025, 1, 1);
        params.not_after = date_time_ymd(2075, 1, 1);
        let cert = params
            .self_signed(&key)
            .map_err(|error| format!("Could not sign the certificate: {error}"))?;
        Ok(Self {
            address,
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            cert_der: cert.der().to_vec(),
        })
    }

    pub fn address(&self) -> IpAddr {
        self.address
    }

    /// SHA-256 of the certificate as `AB:CD:…`, the form `openssl x509 -fingerprint -sha256` and
    /// browsers print.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(&self.cert_der);
        let mut text = String::with_capacity(digest.len() * 3);
        for (index, byte) in digest.iter().enumerate() {
            if index > 0 {
                text.push(':');
            }
            text.push_str(&format!("{byte:02X}"));
        }
        text
    }

    /// The certificate as a client's trust store takes it.
    #[cfg(test)]
    pub fn certificate_der(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.cert_der.clone())
    }

    /// The rustls configuration for a listener: TLS 1.3, HTTP/1.1, no client certificates (the
    /// bearer token authenticates the client).
    pub fn server_config(&self) -> Result<Arc<ServerConfig>, String> {
        let cert = CertificateDer::from(self.cert_der.clone());
        let key = PrivateKeyDer::from_pem_slice(self.key_pem.as_bytes())
            .map_err(|error| format!("The TLS key is unreadable: {error}"))?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| format!("Could not configure TLS: {error}"))?
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .map_err(|error| format!("The TLS certificate does not fit its key: {error}"))?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }

    fn to_file_text(&self) -> String {
        format!(
            "{ADDRESS_PREFIX}{}\n{}{}",
            self.address, self.cert_pem, self.key_pem
        )
    }

    fn from_file_text(text: &str) -> Result<Self, String> {
        let first = text.lines().next().unwrap_or_default();
        let address: IpAddr = first
            .strip_prefix(ADDRESS_PREFIX)
            .and_then(|rest| rest.trim().parse().ok())
            .ok_or("The TLS file has no address line")?;
        let cert = CertificateDer::from_pem_slice(text.as_bytes())
            .map_err(|error| format!("The TLS certificate is unreadable: {error}"))?;
        let identity = Self {
            address,
            cert_pem: pem_block(text, "CERTIFICATE").ok_or("The TLS file has no certificate")?,
            key_pem: pem_block(text, "PRIVATE KEY").ok_or("The TLS file has no key")?,
            cert_der: cert.to_vec(),
        };
        // Reject a certificate and key that were pulled apart before they are ever served.
        identity.server_config()?;
        Ok(identity)
    }
}

/// The PEM block labelled `label`, including its BEGIN and END lines and a final newline.
fn pem_block(text: &str, label: &str) -> Option<String> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let from = text.find(&begin)?;
    let to = text[from..].find(&end)? + from + end.len();
    Some(format!("{}\n", &text[from..to]))
}

/// Where the identity is kept inside `dir`.
pub fn path_in(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// The identity saved in `dir` if it was made for `address`; otherwise a new one, saved before it
/// is returned. A new address means a new certificate and so a new fingerprint to pin.
///
/// A file that exists but cannot be read is an error, not a reason to replace it: the user would
/// otherwise see their pinned fingerprint change for no stated reason.
pub fn load_or_create(dir: &Path, address: IpAddr) -> Result<TlsIdentity, String> {
    let path = path_in(dir);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let saved = TlsIdentity::from_file_text(&text)
                .map_err(|error| format!("{error} ({})", path.display()))?;
            if saved.address == address {
                return Ok(saved);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    }
    let fresh = TlsIdentity::generate(address)?;
    save(&path, &fresh)?;
    Ok(fresh)
}

/// Writes the identity owner-only, through a temporary file renamed over the target, so the key
/// is never readable by others and a crash never leaves half a file.
fn save(path: &Path, identity: &TlsIdentity) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().ok_or("The TLS file has no directory")?;
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    let temp = path.with_extension("pem.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write = || -> std::io::Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(identity.to_file_text().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    };
    write().map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        format!("Could not save {}: {error}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address() -> IpAddr {
        "192.168.1.20".parse().unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "degenview-mcp-tls-{name}-{}",
            crate::model::watchlist::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_generated_identity_builds_a_server_config() {
        let identity = TlsIdentity::generate(address()).unwrap();
        let config = identity.server_config().unwrap();
        assert_eq!(config.alpn_protocols, vec![b"http/1.1".to_vec()]);
        // 32 bytes as `AB:CD:…`.
        let fingerprint = identity.fingerprint();
        assert_eq!(fingerprint.len(), 32 * 3 - 1);
        assert!(
            fingerprint
                .chars()
                .all(|c| c == ':' || c.is_ascii_digit() || ('A'..='F').contains(&c))
        );
    }

    #[test]
    fn two_identities_are_different_and_debug_hides_the_key() {
        let a = TlsIdentity::generate(address()).unwrap();
        let b = TlsIdentity::generate(address()).unwrap();
        assert_ne!(a.fingerprint(), b.fingerprint());
        let shown = format!("{a:?}");
        assert!(!shown.contains("PRIVATE"), "{shown}");
        assert!(!shown.contains(&a.key_pem));
    }

    #[test]
    fn the_same_address_loads_the_same_identity() {
        let dir = scratch("same");
        let first = load_or_create(&dir, address()).unwrap();
        let second = load_or_create(&dir, address()).unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(first.cert_der, second.cert_der);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_address_gets_a_new_certificate() {
        let dir = scratch("moved");
        let first = load_or_create(&dir, address()).unwrap();
        let moved = load_or_create(&dir, "10.0.0.7".parse().unwrap()).unwrap();
        assert_ne!(first.fingerprint(), moved.fingerprint());
        assert_eq!(moved.address(), "10.0.0.7".parse::<IpAddr>().unwrap());
        // And the new one is what is saved now.
        let again = load_or_create(&dir, "10.0.0.7".parse().unwrap()).unwrap();
        assert_eq!(moved.fingerprint(), again.fingerprint());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_is_reported_and_left_alone() {
        let dir = scratch("damaged");
        let path = path_in(&dir);
        std::fs::write(&path, "not a certificate").unwrap();
        let error = load_or_create(&dir, address()).unwrap_err();
        assert!(error.contains("mcp-tls.pem"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a certificate");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_certificate_paired_with_another_key_is_refused() {
        let a = TlsIdentity::generate(address()).unwrap();
        let b = TlsIdentity::generate(address()).unwrap();
        let mixed = format!("{ADDRESS_PREFIX}{}\n{}{}", address(), a.cert_pem, b.key_pem);
        assert!(TlsIdentity::from_file_text(&mixed).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        load_or_create(&dir, address()).unwrap();
        let mode = std::fs::metadata(path_in(&dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
