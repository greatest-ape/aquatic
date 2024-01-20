use std::{fs::File, io::BufReader, path::Path};

use anyhow::Context;

pub type RustlsConfig = rustls::ServerConfig;

pub fn create_rustls_config(
    tls_certificate_path: &Path,
    tls_private_key_path: &Path,
) -> anyhow::Result<RustlsConfig> {
    let certs = {
        let f = File::open(tls_certificate_path).with_context(|| {
            format!(
                "open tls certificate file at {}",
                tls_certificate_path.to_string_lossy()
            )
        })?;
        let mut f = BufReader::new(f);

        let mut certs = Vec::new();

        for cert in rustls_pemfile::certs(&mut f) {
            match cert {
                Ok(cert) => {
                    certs.push(cert);
                }
                Err(err) => {
                    ::log::error!("error parsing certificate: {:#?}", err)
                }
            }
        }

        certs
    };

    let private_key = {
        let f = File::open(tls_private_key_path).with_context(|| {
            format!(
                "open tls private key file at {}",
                tls_private_key_path.to_string_lossy()
            )
        })?;
        let mut f = BufReader::new(f);

        let key = rustls_pemfile::pkcs8_private_keys(&mut f)
            .next()
            .ok_or(anyhow::anyhow!("No private keys in file"))??;

        #[allow(clippy::let_and_return)] // Using temporary variable fixes lifetime issue
        key
    };

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, rustls::pki_types::PrivateKeyDer::Pkcs8(private_key))
        .with_context(|| "create rustls config")?;

    Ok(tls_config)
}
