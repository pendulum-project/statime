use rustls::{crypto::{CryptoProvider, WebPkiSupportedAlgorithms}, server::danger::ClientCertVerified};

#[derive(Debug)]
pub struct OnlyAllowedClients {
    supported_algs: WebPkiSupportedAlgorithms,
    allowed_clients: Vec<rustls::pki_types::CertificateDer<'static>>,
}

impl OnlyAllowedClients {
    pub fn new(provider: CryptoProvider, allowed_clients: Vec<rustls::pki_types::CertificateDer<'static>>) -> Self {
        OnlyAllowedClients {
            supported_algs: provider.signature_verification_algorithms,
            allowed_clients,
        }
    }
}

impl rustls::server::danger::ClientCertVerifier for OnlyAllowedClients {
    fn verify_client_cert(
            &self,
            end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
            if self.allowed_clients.iter().any(|c| c == end_entity) {
                Ok(ClientCertVerified::assertion())
            } else {
                Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
            }
    }

    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}
