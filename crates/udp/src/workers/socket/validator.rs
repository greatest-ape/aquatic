use std::net::IpAddr;
use std::time::Instant;

use anyhow::Context;
use constant_time_eq::constant_time_eq;
use getrandom::getrandom;

use aquatic_common::CanonicalSocketAddr;
use aquatic_udp_protocol::ConnectionId;

use crate::config::Config;

/// HMAC (BLAKE3) based ConnectionID creator and validator
///
/// Structure of created ConnectionID (bytes making up inner i64):
/// - &[0..4]: connection expiration time as number of seconds after
///   ConnectionValidator instance was created, encoded as u32 bytes.
///   Value fits around 136 years.
/// - &[4..8]: truncated keyed BLAKE3 hash of above 4 bytes and octets of
///   client IP address
///
/// The purpose of using ConnectionIDs is to prevent IP spoofing, mainly to
/// prevent the tracker from being used as an amplification vector for DDoS
/// attacks. By including 32 bits of BLAKE3 keyed hash output in its contents,
/// such abuse should be rendered impractical.
#[derive(Clone)]
pub struct ConnectionValidator {
    start_time: Instant,
    max_connection_age: u32,
    keyed_hasher: blake3::Hasher,
}

impl ConnectionValidator {
    /// Create new instance. Must be created once and cloned if used in several
    /// threads.
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let mut key = [0; 32];

        getrandom(&mut key)
            .with_context(|| "Couldn't get random bytes for ConnectionValidator key")?;

        let keyed_hasher = blake3::Hasher::new_keyed(&key);

        Ok(Self {
            keyed_hasher,
            start_time: Instant::now(),
            max_connection_age: config.cleaning.max_connection_age,
        })
    }

    pub fn create_connection_id(&mut self, source_addr: CanonicalSocketAddr) -> ConnectionId {
        let valid_until =
            (self.start_time.elapsed().as_secs() as u32 + self.max_connection_age).to_ne_bytes();

        let hash = self.hash(valid_until, source_addr.get().ip());

        let mut connection_id_bytes = [0u8; 8];

        (&mut connection_id_bytes[..4]).copy_from_slice(&valid_until);
        (&mut connection_id_bytes[4..]).copy_from_slice(&hash);

        ConnectionId(i64::from_ne_bytes(connection_id_bytes))
    }

    pub fn connection_id_valid(
        &mut self,
        source_addr: CanonicalSocketAddr,
        connection_id: ConnectionId,
    ) -> bool {
        let bytes = connection_id.0.to_ne_bytes();
        let (valid_until, hash) = bytes.split_at(4);
        let valid_until: [u8; 4] = valid_until.try_into().unwrap();

        if !constant_time_eq(hash, &self.hash(valid_until, source_addr.get().ip())) {
            return false;
        }

        u32::from_ne_bytes(valid_until) > self.start_time.elapsed().as_secs() as u32
    }

    fn hash(&mut self, valid_until: [u8; 4], ip_addr: IpAddr) -> [u8; 4] {
        self.keyed_hasher.update(&valid_until);

        match ip_addr {
            IpAddr::V4(ip) => self.keyed_hasher.update(&ip.octets()),
            IpAddr::V6(ip) => self.keyed_hasher.update(&ip.octets()),
        };

        let mut hash = [0u8; 4];

        self.keyed_hasher.finalize_xof().fill(&mut hash);
        self.keyed_hasher.reset();

        hash
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use quickcheck_macros::quickcheck;

    use super::*;

    #[quickcheck]
    fn test_connection_validator(
        original_addr: IpAddr,
        different_addr: IpAddr,
        max_connection_age: u32,
    ) -> quickcheck::TestResult {
        let original_addr = CanonicalSocketAddr::new(SocketAddr::new(original_addr, 0));
        let different_addr = CanonicalSocketAddr::new(SocketAddr::new(different_addr, 0));

        if original_addr == different_addr {
            return quickcheck::TestResult::discard();
        }

        let mut validator = {
            let mut config = Config::default();

            config.cleaning.max_connection_age = max_connection_age;

            ConnectionValidator::new(&config).unwrap()
        };

        let connection_id = validator.create_connection_id(original_addr);

        let original_valid = validator.connection_id_valid(original_addr, connection_id);
        let different_valid = validator.connection_id_valid(different_addr, connection_id);

        if different_valid {
            return quickcheck::TestResult::failed();
        }

        if max_connection_age == 0 {
            quickcheck::TestResult::from_bool(!original_valid)
        } else {
            // Note: depends on that running this test takes less than a second
            quickcheck::TestResult::from_bool(original_valid)
        }
    }
}
