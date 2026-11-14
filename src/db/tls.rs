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
                 servidor y lo firma una autoridad pública o la del fichero\n\
                 de CA que indiques."
            }
        }
    }

    /// Si el certificado del servidor se da por bueno sin mirarlo.
    pub fn accepts_any_certificate(self) -> bool {
        self != TlsMode::Verify
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

/// Configuración de rustls para `mode`. Al verificar se confía en las
/// autoridades públicas y, si se indica, en las del fichero PEM `ca_file`.
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
    if let Some(path) = ca_file {
        let unreadable =
            |e: &dyn std::fmt::Display| format!("No se pudo leer el fichero de CA «{path}»: {e}");
        let certs = CertificateDer::pem_file_iter(path).map_err(|e| unreadable(&e))?;
        let mut added = 0;
        for cert in certs {
            roots
                .add(cert.map_err(|e| unreadable(&e))?)
                .map_err(|e| unreadable(&e))?;
            added += 1;
        }
        if added == 0 {
            return Err(unreadable(&"no contiene ningún certificado en formato PEM"));
        }
    }
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
