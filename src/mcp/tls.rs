//! The certificate a server on a named network interface presents.
//!
//! Loopback needs none. A server reachable from other machines is served over TLS only, with a
//! self-signed certificate made here for the one address it listens on. Nothing vouches for a
//! self-signed certificate, so the client pins it: [`TlsIdentity::fingerprint`] is what the user
//! compares (or the client stores) the first time it connects.
//!
//! The certificate and its private key live in one PEM file in the app's data directory, written
//! owner-only on Unix and put back to owner-only if a restore or a copy loosened it. The key is not
//! part of the credential seal. A process that can only replace the file can present another
//! certificate, which a client that pinned the fingerprint refuses. A process that can *read* the key
//! is worse off: it can pose as the server to such a client and take the bearer token it sends, so
//! the file is never left readable by others. A loaded file is trusted for nothing it says about
//! itself: the address it is kept for must be one the certificate names. TLS 1.3 only; the provider is
//! `ring`, chosen explicitly so no process-wide default has to be installed.

use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, date_time_ymd,
};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio_rustls::rustls::{self, ServerConfig};

/// The file inside the data directory.
pub const FILE_NAME: &str = "mcp-tls.pem";

/// Held, exclusively, for the whole of reading, making and saving the identity, so two DegenView
/// processes starting together adopt the same certificate instead of each making one.
pub const LOCK_FILE_NAME: &str = "mcp-tls.lock";

/// How long [`load_or_create`] waits for another process that is making the certificate. Making
/// one takes milliseconds, so a longer wait means that process is stuck, and the caller is told.
const LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
        let identity = Self {
            address,
            cert_pem: cert.pem(),
            key_pem: key.serialize_pem(),
            cert_der: cert.der().to_vec(),
        };
        names_address(&identity.cert_der, address)?;
        Ok(identity)
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
        // Reject a certificate and key that were pulled apart before they are ever served, and a
        // file whose address line says one thing while the certificate names another.
        identity.server_config()?;
        names_address(&identity.cert_der, address)?;
        Ok(identity)
    }
}

/// Whether the certificate names `address` (an IP subject alternative name), checked as a client
/// checks it. The address a file records about itself is only a note: this is what it is held to.
fn names_address(cert_der: &[u8], address: IpAddr) -> Result<(), String> {
    let der = CertificateDer::from(cert_der);
    let cert = webpki::EndEntityCert::try_from(&der)
        .map_err(|error| format!("The TLS certificate is not usable: {error}"))?;
    cert.verify_is_valid_for_subject_name(&ServerName::IpAddress(address.into()))
        .map_err(|_| format!("The TLS certificate does not name {address}"))
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
///
/// The whole operation runs under a lock on [`LOCK_FILE_NAME`], shared with every other process
/// using the directory. Without it, two instances that find no file both make a certificate: each
/// replaces the other's temporary file, one fails, and the other can end up presenting a
/// certificate that is not the one saved. An advisory lock is released when its holder exits or
/// crashes, so it cannot be left behind.
pub fn load_or_create(dir: &Path, address: IpAddr) -> Result<TlsIdentity, String> {
    load_or_create_within(dir, address, LOCK_TIMEOUT)
}

fn load_or_create_within(
    dir: &Path,
    address: IpAddr,
    wait: std::time::Duration,
) -> Result<TlsIdentity, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    let _held = lock_dir(dir, wait)?;
    load_or_create_locked(dir, address)
}

/// An exclusive lock on the directory's lock file, held until it is dropped.
fn lock_dir(dir: &Path, wait: std::time::Duration) -> Result<std::fs::File, String> {
    let path = dir.join(LOCK_FILE_NAME);
    let mut options = std::fs::OpenOptions::new();
    // Read and write, for the systems whose locks need one of them.
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let started = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < wait => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "Another DegenView has held {} for {} seconds; try again in a moment",
                    path.display(),
                    wait.as_secs().max(1)
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(format!("Could not lock {}: {error}", path.display()));
            }
        }
    }
}

fn load_or_create_locked(dir: &Path, address: IpAddr) -> Result<TlsIdentity, String> {
    let path = path_in(dir);
    ensure_private(&path)?;
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

/// Makes sure an existing key file is a plain file only its owner can read. A restore or a copy
/// often leaves it readable by others (a `cp` without `-p`), so a loose mode is put back to
/// owner-only rather than used as it is; a link or anything that is not a regular file is refused,
/// since reading through it would read something else. Nothing to do when there is no file, and on
/// systems without Unix modes (there the per-user data directory is what protects it).
fn ensure_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
        };
        if !meta.file_type().is_file() {
            return Err(format!(
                "{} is not a regular file, so it was not used",
                path.display()
            ));
        }
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |error| {
                    format!(
                        "{} can be read by others and could not be made private: {error}",
                        path.display()
                    )
                },
            )?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Writes the identity owner-only, through a new temporary file renamed over the target, so the
/// key is never readable by others and a crash never leaves half a file. The temporary file is
/// always created fresh: one left behind (or planted, as a link) is removed, never reused, because
/// the mode asked for only applies to a file that is being created.
fn save(path: &Path, identity: &TlsIdentity) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().ok_or("The TLS file has no directory")?;
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    let temp = path.with_extension("pem.tmp");
    match std::fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("Could not clear {}: {error}", temp.display())),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let write = || -> std::io::Result<()> {
        let mut file = options.open(&temp)?;
        // Not left to the umask, and before the key is written.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
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

    /// A certificate that names `names` (DNS or IP), with the key for it, as a file text whose
    /// address line says `recorded`.
    fn file_text_for(recorded: &str, names: &[&str]) -> String {
        let key = KeyPair::generate().unwrap();
        let params =
            CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>())
                .unwrap();
        let cert = params.self_signed(&key).unwrap();
        format!(
            "{ADDRESS_PREFIX}{recorded}\n{}{}",
            cert.pem(),
            key.serialize_pem()
        )
    }

    #[test]
    fn an_address_line_the_certificate_does_not_name_is_refused() {
        // What the file says about itself is only a note: the certificate has to name the address.
        for (recorded, names) in [
            ("192.168.1.20", &["10.0.0.7"][..]),
            ("fd00::21", &["fd00::20"][..]),
            ("192.168.1.20", &["example.test"][..]),
            ("192.168.1.20", &["10.0.0.7", "10.0.0.8"][..]),
        ] {
            let error = TlsIdentity::from_file_text(&file_text_for(recorded, names)).unwrap_err();
            assert!(
                error.contains("does not name"),
                "{recorded} / {names:?}: {error}"
            );
        }
        // And the honest cases load: one address, or several that include it.
        for (recorded, names) in [
            ("192.168.1.20", &["192.168.1.20"][..]),
            ("fd00::20", &["fd00::20"][..]),
            ("192.168.1.20", &["10.0.0.7", "192.168.1.20"][..]),
        ] {
            let identity = TlsIdentity::from_file_text(&file_text_for(recorded, names)).unwrap();
            assert_eq!(identity.address(), recorded.parse::<IpAddr>().unwrap());
        }
    }

    #[test]
    fn a_mismatched_file_is_reported_by_load_or_create_and_left_alone() {
        let dir = scratch("san");
        let path = path_in(&dir);
        let text = file_text_for("192.168.1.20", &["10.0.0.7"]);
        std::fs::write(&path, &text).unwrap();
        let error = load_or_create(&dir, "192.168.1.20".parse().unwrap()).unwrap_err();
        assert!(
            error.contains("does not name") && error.contains("mcp-tls.pem"),
            "{error}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_a_restore_made_readable_by_others_is_made_private_and_kept() {
        let dir = scratch("loose");
        let first = load_or_create(&dir, address()).unwrap();
        for loose in [0o644, 0o664, 0o640, 0o666, 0o604] {
            set_mode(&path_in(&dir), loose);
            let again = load_or_create(&dir, address()).unwrap();
            assert_eq!(mode_of(&path_in(&dir)), 0o600, "from {loose:o}");
            assert_eq!(
                again.fingerprint(),
                first.fingerprint(),
                "the certificate is kept"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_temporary_file_left_loose_is_never_reused_for_the_key() {
        let dir = scratch("tmp");
        let temp = path_in(&dir).with_extension("pem.tmp");
        std::fs::write(&temp, "left behind").unwrap();
        set_mode(&temp, 0o666);
        load_or_create(&dir, address()).unwrap();
        assert_eq!(mode_of(&path_in(&dir)), 0o600);
        assert!(!temp.exists(), "the temporary file is renamed away");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_planted_where_the_key_is_written_is_not_written_through() {
        let dir = scratch("link");
        let victim = dir.join("victim");
        std::fs::write(&victim, "keep me").unwrap();
        let temp = path_in(&dir).with_extension("pem.tmp");
        std::os::unix::fs::symlink(&victim, &temp).unwrap();
        load_or_create(&dir, address()).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        let meta = std::fs::symlink_metadata(path_in(&dir)).unwrap();
        assert!(meta.file_type().is_file(), "the key is a plain file");
        assert_eq!(mode_of(&path_in(&dir)), 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_that_is_a_link_is_refused() {
        let dir = scratch("keylink");
        load_or_create(&dir, address()).unwrap();
        let real = dir.join("elsewhere.pem");
        std::fs::rename(path_in(&dir), &real).unwrap();
        std::os::unix::fs::symlink(&real, path_in(&dir)).unwrap();
        let error = load_or_create(&dir, address()).unwrap_err();
        assert!(error.contains("not a regular file"), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn callers_starting_together_all_adopt_the_one_certificate_that_is_saved() {
        use std::sync::{Arc, Barrier};
        // As several DegenView processes finding no file at once would: every caller must succeed
        // and hold exactly the certificate that ended up on disk. (Before the lock, two callers
        // never got through a round: one failed, and others held a certificate that was not saved.)
        for callers in [2, 3, 4] {
            for round in 0..60 {
                let dir = scratch(&format!("race{callers}-{round}"));
                let barrier = Arc::new(Barrier::new(callers));
                let handles: Vec<_> = (0..callers)
                    .map(|_| {
                        let (dir, barrier) = (dir.clone(), barrier.clone());
                        std::thread::spawn(move || {
                            barrier.wait();
                            load_or_create(&dir, address()).map(|identity| identity.fingerprint())
                        })
                    })
                    .collect();
                let held: Vec<String> = handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap().expect("every caller succeeds"))
                    .collect();
                let saved = load_or_create(&dir, address()).unwrap().fingerprint();
                assert!(
                    held.iter().all(|fingerprint| *fingerprint == saved),
                    "{callers} callers, round {round}: {held:?} vs saved {saved}"
                );
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
    }

    #[test]
    fn a_lock_another_process_holds_is_reported_after_a_wait_not_waited_on_forever() {
        let dir = scratch("held");
        std::fs::create_dir_all(&dir).unwrap();
        // Another holder: its own handle on the lock file, as another process would have.
        let holder = lock_dir(&dir, std::time::Duration::from_millis(100)).unwrap();
        let started = std::time::Instant::now();
        let error = load_or_create_within(&dir, address(), std::time::Duration::from_millis(200))
            .unwrap_err();
        assert!(
            error.contains("Another DegenView") && error.contains(LOCK_FILE_NAME),
            "{error}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "it must not hang"
        );
        assert!(
            !path_in(&dir).exists(),
            "nothing is made while someone else holds the lock"
        );

        // Released, the same call goes through.
        drop(holder);
        assert!(load_or_create(&dir, address()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_lock_is_let_go_after_every_call_and_the_lock_file_is_private() {
        let dir = scratch("release");
        load_or_create(&dir, address()).unwrap();
        // Not held now: a try_lock from here succeeds at once, and again after an error path.
        let probe = lock_dir(&dir, std::time::Duration::from_millis(100));
        assert!(probe.is_ok(), "the lock was left held");
        drop(probe);
        std::fs::write(path_in(&dir), "damaged").unwrap();
        assert!(load_or_create(&dir, address()).is_err());
        assert!(
            lock_dir(&dir, std::time::Duration::from_millis(100)).is_ok(),
            "an error left the lock held"
        );
        #[cfg(unix)]
        assert_eq!(mode_of(&dir.join(LOCK_FILE_NAME)), 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
