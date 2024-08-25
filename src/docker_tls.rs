use std::net::Ipv4Addr;

use rcgen::{Certificate, CertificateParams, CertifiedKey, ExtendedKeyUsagePurpose, KeyPair};

pub struct DockerCA {
    key_pair: KeyPair,
    pub cert: Certificate,
}

impl DockerCA {
    pub fn new() -> anyhow::Result<Self> {
        let key_pair = KeyPair::generate()?;

        let mut cert_params = CertificateParams::new([])?;
        cert_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let cert = cert_params.self_signed(&key_pair)?;

        Ok(Self { key_pair, cert })
    }

    pub fn create_server_cert(&self, ip: Ipv4Addr) -> anyhow::Result<CertifiedKey> {
        let key_pair = KeyPair::generate()?;

        let mut cert_params = CertificateParams::new([ip.to_string()])?;
        cert_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let cert = cert_params.signed_by(&key_pair, &self.cert, &self.key_pair)?;

        Ok(CertifiedKey { cert, key_pair })
    }

    pub fn create_client_cert(&self) -> anyhow::Result<CertifiedKey> {
        let key_pair = KeyPair::generate()?;

        let mut cert_params = CertificateParams::new([])?;
        cert_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = cert_params.signed_by(&key_pair, &self.cert, &self.key_pair)?;

        Ok(CertifiedKey { cert, key_pair })
    }
}
