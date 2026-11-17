use crate::cert::format_fingerprint;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct PinnedCertVerifier {
    trusted_fingerprints: Arc<Mutex<Vec<String>>>,
    discovered_fingerprint: Arc<Mutex<Option<String>>>,
}

impl PinnedCertVerifier {
    pub fn new(
        trusted_fingerprints: Arc<Mutex<Vec<String>>>,
        discovered_fingerprint: Arc<Mutex<Option<String>>>,
    ) -> Self {
        Self {
            trusted_fingerprints,
            discovered_fingerprint,
        }
    }
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let fp = format_fingerprint(end_entity.as_ref());
        *self.discovered_fingerprint.lock().unwrap() = Some(fp.clone());

        let trusted = self.trusted_fingerprints.lock().unwrap();
        // If trusted list is empty (first connect / discovery), accept on first use (TOFU)
        if trusted.is_empty() || trusted.contains(&fp) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}
