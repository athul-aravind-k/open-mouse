use ring::digest::{digest, SHA256};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::fs;
use std::path::PathBuf;

pub fn get_certs_dir() -> PathBuf {
    if let Ok(app_data) = std::env::var("APPDATA") {
        PathBuf::from(app_data).join("OpenMouse").join("certs")
    } else {
        PathBuf::from(".").join(".openmouse").join("certs")
    }
}

pub fn format_fingerprint(cert_der: &[u8]) -> String {
    let hash = digest(&SHA256, cert_der);
    hash.as_ref()
        .iter()
        .map(|b| format!("{:02X}", b))
        .collect::<Vec<String>>()
        .join(":")
}

pub fn get_or_create_certificate(
    device_name: &str,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>, String), String> {
    let dir = get_certs_dir();
    let cert_path = dir.join("cert.der");
    let key_path = dir.join("key.der");

    if cert_path.exists() && key_path.exists() {
        if let (Ok(cert_bytes), Ok(key_bytes)) = (fs::read(&cert_path), fs::read(&key_path)) {
            let fingerprint = format_fingerprint(&cert_bytes);
            let cert = CertificateDer::from(cert_bytes);
            let key = PrivateKeyDer::try_from(key_bytes)
                .map_err(|e| format!("Invalid private key DER: {:?}", e))?;
            return Ok((cert, key, fingerprint));
        }
    }

    // Generate new self-signed certificate
    let subject_alt_names = vec![device_name.to_string(), "open-mouse".to_string(), "localhost".to_string()];
    let cert_key = rcgen::generate_simple_self_signed(subject_alt_names)
        .map_err(|e| format!("Certificate generation failed: {}", e))?;

    let cert_der = cert_key.cert.der().to_vec();
    let key_der = cert_key.key_pair.serialize_der();

    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(&cert_path, &cert_der);
    let _ = fs::write(&key_path, &key_der);

    let fingerprint = format_fingerprint(&cert_der);
    let cert = CertificateDer::from(cert_der);
    let key = PrivateKeyDer::try_from(key_der)
        .map_err(|e| format!("Invalid generated private key DER: {:?}", e))?;

    Ok((cert, key, fingerprint))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cert_generation_and_fingerprint() {
        let (cert, _key, fp) = get_or_create_certificate("TestLaptop").expect("Cert generation failed");
        assert!(!cert.is_empty());
        assert!(!fp.is_empty());
        assert_eq!(fp.split(':').count(), 32); // SHA-256 is 32 bytes

        let fp_check = format_fingerprint(&cert);
        assert_eq!(fp, fp_check);
    }
}
