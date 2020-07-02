pub mod stream;

use std::fs::File;
use std::io::Read;

use anyhow::Context;
use native_tls::{Identity, TlsAcceptor};

use crate::config::TlsConfig;


pub fn create_tls_acceptor(
    config: &TlsConfig,
) -> anyhow::Result<Option<TlsAcceptor>> {
    if config.use_tls {
        let mut identity_bytes = Vec::new();
        let mut file = File::open(&config.tls_pkcs12_path)
            .context("Couldn't open pkcs12 identity file")?;

        file.read_to_end(&mut identity_bytes)
            .context("Couldn't read pkcs12 identity file")?;

        let identity = Identity::from_pkcs12(
            &mut identity_bytes,
            &config.tls_pkcs12_password
        ).context("Couldn't parse pkcs12 identity file")?;

        let acceptor = TlsAcceptor::new(identity)
            .context("Couldn't create TlsAcceptor from pkcs12 identity")?;

        Ok(Some(acceptor))
    } else {
        Ok(None)
    }
}