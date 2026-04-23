use std::fs;
use std::sync::Arc;

use crate::options::Options;

pub(crate) fn make_tls_config(opts: &Options) -> Result<Arc<rustls::ClientConfig>, String> {
    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ref ca_path) = opts.cacert {
        let pem = fs::read(ca_path).map_err(|e| format!("cacert: failed to read cacert: {e}"))?;
        let certs = rustls_pemfile::certs(&mut &pem[..])
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("cacert: failed to parse cacert PEM: {e}"))?;
        if certs.is_empty() {
            return Err(format!(
                "cacert: no certificates found in {}",
                ca_path.display()
            ));
        }
        for cert in certs {
            root_store
                .add(cert)
                .map_err(|e| format!("cacert: failed to add CA cert: {e}"))?;
        }
    } else {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        // Also try native certs.
        let native = rustls_native_certs::load_native_certs();
        for cert in native.certs {
            let _ = root_store.add(cert);
        }
    }

    let builder = rustls::ClientConfig::builder().with_root_certificates(root_store);

    let config = if let Some(ref cert_path) = opts.cert {
        let cert_pem =
            fs::read(cert_path).map_err(|e| format!("failed to read client cert: {e}"))?;
        let certs = rustls_pemfile::certs(&mut &cert_pem[..])
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("failed to parse client cert PEM: {e}"))?;

        let key_path = opts.cert_key.as_ref().unwrap_or(cert_path);
        let key_pem = fs::read(key_path).map_err(|e| format!("failed to read client key: {e}"))?;
        let key = rustls_pemfile::private_key(&mut &key_pem[..])
            .map_err(|e| format!("failed to parse client key PEM: {e}"))?
            .ok_or_else(|| "no private key found in PEM".to_string())?;

        builder
            .with_client_auth_cert(certs, key)
            .map_err(|e| format!("client auth setup failed: {e}"))?
    } else {
        builder.with_no_client_auth()
    };

    Ok(Arc::new(config))
}

/// A verifier that accepts any certificate (for -k / --insecure).
#[derive(Debug)]
pub(crate) struct InsecureVerifier;

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::ED448,
        ]
    }
}
