//! Connection security: whether it is encrypted with TLS and whether the
//! other end is checked. It is the same for every engine.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};

use super::DbResult;
use crate::i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// Encrypts if the server offers it; otherwise connects unencrypted. Does
    /// not check the certificate (libpq's `sslmode=prefer`).
    #[default]
    Prefer,
    /// Does not connect without TLS. Does not check the certificate: protects
    /// against eavesdroppers, not against someone impersonating the server.
    Require,
    /// Does not connect without TLS, and the certificate must belong to the
    /// server being called and be signed by a trusted authority.
    Verify,
}

impl TlsMode {
    pub const ALL: [TlsMode; 3] = [TlsMode::Prefer, TlsMode::Require, TlsMode::Verify];

    pub fn label(self) -> String {
        match self {
            TlsMode::Prefer => t!("tls-prefer"),
            TlsMode::Require => t!("tls-require"),
            TlsMode::Verify => t!("tls-verify"),
        }
    }

    pub fn description(self) -> String {
        match self {
            TlsMode::Prefer => t!("tls-prefer-description"),
            TlsMode::Require => t!("tls-require-description"),
            TlsMode::Verify => t!("tls-verify-description"),
        }
    }

    /// Whether the server certificate is accepted without looking at it.
    pub fn accepts_any_certificate(self) -> bool {
        self != TlsMode::Verify
    }
}

/// How an open connection really travels, to show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Encrypted with TLS.
    Encrypted,
    /// Unencrypted, but it never leaves this machine (socket or local address).
    Local,
    /// Unencrypted and over the network.
    Plain,
}

impl Security {
    /// `encrypted`: what the server says about the connection. `host`: where
    /// it connected to.
    pub fn of(encrypted: bool, host: &str) -> Self {
        let host = host.trim();
        let local = host.starts_with('/')
            || host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        match (encrypted, local) {
            (true, _) => Security::Encrypted,
            (false, true) => Security::Local,
            (false, false) => Security::Plain,
        }
    }

    /// Explanation for whoever hovers over the padlock.
    pub fn description(self, mode: TlsMode) -> String {
        match (self, mode) {
            (Security::Encrypted, TlsMode::Verify) => t!("security-verified"),
            (Security::Encrypted, _) => t!("security-encrypted", verify = TlsMode::Verify.label()),
            (Security::Local, _) => t!("security-local"),
            (Security::Plain, _) => t!("security-plain"),
        }
    }
}

/// What to add to a connection error so it is clear what to do, if it is
/// a TLS one. `error` is the driver's text.
pub fn hint(mode: TlsMode, error: &str) -> String {
    let lower = error.to_lowercase();
    if mode == TlsMode::Verify && (lower.contains("certificate") || lower.contains("cert")) {
        format!("\n\n{}", t!("tls-hint-certificate"))
    } else if mode != TlsMode::Prefer
        && ["tls", "ssl", "secure connection"]
            .iter()
            .any(|word| lower.contains(word))
    {
        format!(
            "\n\n{}",
            t!("tls-hint-required", prefer = TlsMode::Prefer.label())
        )
    } else {
        String::new()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The MySQL crate uses the process's default crypto provider: the same
/// one the rest uses is installed (the first time; afterwards it does nothing).
pub fn install_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Accepts any certificate: the connection is encrypted but the other end
/// is not checked. Lets users into servers with a self-signed certificate,
/// which are most of them.
#[derive(Debug)]
struct AnyCertificate(Arc<CryptoProvider>);

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Authorities trusted when verifying, besides the public ones: those
/// installed on the system and, if given, those of the PEM file
/// `ca_file`. Only those rustls can read (a system store may include
/// some it cannot).
pub fn trusted_authorities(ca_file: Option<&str>) -> DbResult<Vec<CertificateDer<'static>>> {
    // If the system has no store or it cannot be read, the public ones remain
    let mut authorities = rustls_native_certs::load_native_certs().certs;
    if let Some(path) = ca_file {
        let unreadable =
            |e: &dyn std::fmt::Display| t!("tls-ca-unreadable", path = path, error = e.to_string());
        let before = authorities.len();
        for cert in CertificateDer::pem_file_iter(path).map_err(|e| unreadable(&e))? {
            let cert = cert.map_err(|e| unreadable(&e))?;
            rustls::RootCertStore::empty()
                .add(cert.clone())
                .map_err(|e| unreadable(&e))?;
            authorities.push(cert);
        }
        if authorities.len() == before {
            return Err(unreadable(&t!("tls-ca-empty")));
        }
    }
    authorities.retain(|cert| rustls::RootCertStore::empty().add(cert.clone()).is_ok());
    Ok(authorities)
}

/// The authorities of `trusted_authorities` in a temporary PEM file, for
/// drivers that only accept a file. Deleted when dropped.
pub struct AuthorityBundle(std::path::PathBuf);

impl AuthorityBundle {
    /// `None` if there is no authority to add to the public ones.
    pub fn create(ca_file: Option<&str>) -> DbResult<Option<Self>> {
        use base64::Engine as _;
        use std::sync::atomic::{AtomicU64, Ordering};

        let authorities = trusted_authorities(ca_file)?;
        if authorities.is_empty() {
            return Ok(None);
        }
        let mut pem = String::new();
        for cert in &authorities {
            let encoded = base64::engine::general_purpose::STANDARD.encode(cert);
            pem.push_str("-----BEGIN CERTIFICATE-----\n");
            for line in encoded.as_bytes().chunks(64) {
                pem.push_str(&String::from_utf8_lossy(line));
                pem.push('\n');
            }
            pem.push_str("-----END CERTIFICATE-----\n");
        }
        // One file per connection: each one may carry its own CA
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "rustql-ca-{}-{}.pem",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, pem).map_err(|e| t!("tls-verifier-failed", error = e.to_string()))?;
        Ok(Some(Self(path)))
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for AuthorityBundle {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// rustls configuration for `mode`. When verifying, the public authorities,
/// the system ones and, if given, those of the PEM file `ca_file` are
/// trusted.
pub fn client_config(mode: TlsMode, ca_file: Option<&str>) -> DbResult<rustls::ClientConfig> {
    let provider = provider();
    let builder = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| t!("tls-setup-failed", error = e.to_string()))?;
    if mode.accepts_any_certificate() {
        return Ok(builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCertificate(provider)))
            .with_no_client_auth());
    }
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots.add_parsable_certificates(trusted_authorities(ca_file)?);
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_fichero_de_ca_tiene_que_existir_y_traer_certificados() {
        let missing = client_config(TlsMode::Verify, Some("/no/existe/ca.pem"));
        assert!(missing.unwrap_err().contains("/no/existe/ca.pem"));

        let dir = std::env::temp_dir().join("rustql-tests-tls");
        std::fs::create_dir_all(&dir).unwrap();
        let empty = dir.join("vacio.pem");
        std::fs::write(&empty, "esto no es un certificado\n").unwrap();
        let err = client_config(TlsMode::Verify, empty.to_str()).unwrap_err();
        assert!(err.contains("no certificate"), "{err}");

        // Without verifying, the file is not even looked at
        client_config(TlsMode::Require, Some("/no/existe/ca.pem")).unwrap();
        client_config(TlsMode::Verify, None).unwrap();
    }

    #[test]
    fn las_autoridades_del_sistema_van_en_un_fichero_que_se_borra() {
        // Whether or not the system has a store, it is not an error
        let Some(bundle) = AuthorityBundle::create(None).unwrap() else {
            return;
        };
        let path = bundle.path().to_path_buf();
        // What is written reads back as certificates
        let read = CertificateDer::pem_file_iter(&path).unwrap().count();
        assert_eq!(read, trusted_authorities(None).unwrap().len());
        assert!(read > 0);
        drop(bundle);
        assert!(!path.exists());
    }

    #[test]
    fn sin_cifrar_solo_preocupa_si_sale_del_equipo() {
        assert_eq!(Security::of(true, "db.example.com"), Security::Encrypted);
        assert_eq!(Security::of(true, "127.0.0.1"), Security::Encrypted);
        for local in [
            "127.0.0.1",
            " localhost ",
            "::1",
            "[::1]",
            "/var/run/postgresql",
        ] {
            assert_eq!(Security::of(false, local), Security::Local, "{local}");
        }
        for remote in ["db.example.com", "192.168.1.20", "10.0.0.5"] {
            assert_eq!(Security::of(false, remote), Security::Plain, "{remote}");
        }
    }

    #[test]
    fn la_pista_de_un_error_depende_del_nivel_de_seguridad() {
        let cert = "invalid peer certificate: UnknownIssuer";
        assert!(hint(TlsMode::Verify, cert).contains("CA file"));
        assert!(hint(TlsMode::Require, "server does not support TLS").contains("requires TLS"));
        // That is how the MySQL crate puts it
        let mysql = "Client requires secure connection but server does not have this capability";
        assert!(hint(TlsMode::Verify, mysql).contains("requires TLS"));
        assert_eq!(hint(TlsMode::Prefer, "server does not support TLS"), "");
        assert_eq!(hint(TlsMode::Verify, "Access denied for user"), "");
    }
}
