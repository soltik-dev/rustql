//! Seguridad de la conexión: si va cifrada con TLS y si se comprueba con
//! quién se habla. Es igual para todos los motores.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};

use super::DbResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// Cifra si el servidor lo ofrece; si no, conecta sin cifrar. No
    /// comprueba el certificado (es el `sslmode=prefer` de libpq).
    #[default]
    Prefer,
    /// Sin TLS no conecta. No comprueba el certificado: protege de quien
    /// escucha, no de quien suplanta al servidor.
    Require,
    /// Sin TLS no conecta, y el certificado debe ser del servidor al que se
    /// llama y estar firmado por una autoridad de confianza.
    Verify,
}

impl TlsMode {
    pub const ALL: [TlsMode; 3] = [TlsMode::Prefer, TlsMode::Require, TlsMode::Verify];

    pub fn label(self) -> &'static str {
        match self {
            TlsMode::Prefer => "Cifrar si se puede",
            TlsMode::Require => "Exigir cifrado",
            TlsMode::Verify => "Verificar el servidor",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            TlsMode::Prefer => {
                "Usa TLS si el servidor lo ofrece y, si no, conecta sin cifrar.\n\
                 No comprueba el certificado."
            }
            TlsMode::Require => {
                "No conecta si el servidor no ofrece TLS.\n\
                 No comprueba el certificado: protege de quien escucha la red,\n\
                 no de quien suplanta al servidor."
            }
            TlsMode::Verify => {
                "No conecta sin TLS, y comprueba que el certificado es de ese\n\
                 servidor y lo firma una autoridad en la que confía el sistema\n\
                 o la del fichero de CA que indiques."
            }
        }
    }

    /// Si el certificado del servidor se da por bueno sin mirarlo.
    pub fn accepts_any_certificate(self) -> bool {
        self != TlsMode::Verify
    }
}

/// Cómo viaja de verdad una conexión abierta, para enseñarlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Cifrada con TLS.
    Encrypted,
    /// Sin cifrar, pero no sale de este equipo (socket o dirección local).
    Local,
    /// Sin cifrar y por la red.
    Plain,
}

impl Security {
    /// `encrypted`: lo que dice el servidor de la conexión. `host`: a dónde
    /// se conectó.
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

    /// Explicación para quien pasa el ratón por el candado.
    pub fn description(self, mode: TlsMode) -> &'static str {
        match (self, mode) {
            (Security::Encrypted, TlsMode::Verify) => {
                "Conexión cifrada (TLS), con el certificado del servidor verificado."
            }
            (Security::Encrypted, _) => {
                "Conexión cifrada (TLS). El certificado del servidor no se verifica:\n\
                 para hacerlo, elige «Verificar el servidor» en la conexión."
            }
            (Security::Local, _) => "Conexión sin cifrar, pero no sale de este equipo.",
            (Security::Plain, _) => {
                "Conexión SIN cifrar: lo que se envía y se recibe, contraseña incluida,\n\
                 puede leerse en la red. El servidor no ofrece TLS."
            }
        }
    }
}

/// Lo que hay que añadir a un error de conexión para que se entienda qué
/// hacer, si es de TLS. `error` es el texto del driver.
pub fn hint(mode: TlsMode, error: &str) -> &'static str {
    let lower = error.to_lowercase();
    if mode == TlsMode::Verify && (lower.contains("certificate") || lower.contains("cert")) {
        "\n\nNo se ha podido verificar el certificado del servidor. Si lo firma una autoridad \
         propia, indica su fichero de CA; si no necesitas verificarlo, elige otro nivel de \
         seguridad."
    } else if mode != TlsMode::Prefer
        && ["tls", "ssl", "secure connection"]
            .iter()
            .any(|word| lower.contains(word))
    {
        "\n\nLa seguridad de esta conexión exige TLS. Si el servidor no lo ofrece, elige \
         «Cifrar si se puede»."
    } else {
        ""
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// El crate de MySQL usa el proveedor criptográfico por defecto del proceso:
/// se fija el mismo que usa el resto (la primera vez; después no hace nada).
pub fn install_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Da por bueno cualquier certificado: la conexión va cifrada pero no se
/// comprueba con quién se habla. Permite entrar en los servidores con
/// certificado autofirmado, que son la mayoría.
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

/// Autoridades en las que se confía al verificar, además de las públicas:
/// las instaladas en el sistema y, si se indica, las del fichero PEM
/// `ca_file`. Solo las que rustls sabe leer (un almacén del sistema puede
/// traer alguna que no).
pub fn trusted_authorities(ca_file: Option<&str>) -> DbResult<Vec<CertificateDer<'static>>> {
    // Si el sistema no tiene almacén o no se puede leer, quedan las públicas
    let mut authorities = rustls_native_certs::load_native_certs().certs;
    if let Some(path) = ca_file {
        let unreadable =
            |e: &dyn std::fmt::Display| format!("No se pudo leer el fichero de CA «{path}»: {e}");
        let before = authorities.len();
        for cert in CertificateDer::pem_file_iter(path).map_err(|e| unreadable(&e))? {
            let cert = cert.map_err(|e| unreadable(&e))?;
            rustls::RootCertStore::empty()
                .add(cert.clone())
                .map_err(|e| unreadable(&e))?;
            authorities.push(cert);
        }
        if authorities.len() == before {
            return Err(unreadable(&"no contiene ningún certificado en formato PEM"));
        }
    }
    authorities.retain(|cert| rustls::RootCertStore::empty().add(cert.clone()).is_ok());
    Ok(authorities)
}

/// Las autoridades de `trusted_authorities` en un fichero PEM temporal, para
/// los drivers que solo admiten un fichero. Se borra al soltarlo.
pub struct AuthorityBundle(std::path::PathBuf);

impl AuthorityBundle {
    /// `None` si no hay ninguna autoridad que añadir a las públicas.
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
        // Un fichero por conexión: cada una puede llevar su propia CA
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "rustql-ca-{}-{}.pem",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, pem)
            .map_err(|e| format!("No se pudo preparar la verificación del certificado: {e}"))?;
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

/// Configuración de rustls para `mode`. Al verificar se confía en las
/// autoridades públicas, en las del sistema y, si se indica, en las del
/// fichero PEM `ca_file`.
pub fn client_config(mode: TlsMode, ca_file: Option<&str>) -> DbResult<rustls::ClientConfig> {
    let provider = provider();
    let builder = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("No se pudo preparar TLS: {e}"))?;
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
        assert!(err.contains("ningún certificado"), "{err}");

        // Sin verificar, el fichero ni se mira
        client_config(TlsMode::Require, Some("/no/existe/ca.pem")).unwrap();
        client_config(TlsMode::Verify, None).unwrap();
    }

    #[test]
    fn las_autoridades_del_sistema_van_en_un_fichero_que_se_borra() {
        // Haya o no almacén en el sistema, no es un error
        let Some(bundle) = AuthorityBundle::create(None).unwrap() else {
            return;
        };
        let path = bundle.path().to_path_buf();
        // Lo que se escribe se vuelve a leer como certificados
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
        assert!(hint(TlsMode::Verify, cert).contains("fichero de CA"));
        assert!(hint(TlsMode::Require, "server does not support TLS").contains("exige TLS"));
        // Así lo dice el crate de MySQL
        let mysql = "Client requires secure connection but server does not have this capability";
        assert!(hint(TlsMode::Verify, mysql).contains("exige TLS"));
        assert_eq!(hint(TlsMode::Prefer, "server does not support TLS"), "");
        assert_eq!(hint(TlsMode::Verify, "Access denied for user"), "");
    }
}
