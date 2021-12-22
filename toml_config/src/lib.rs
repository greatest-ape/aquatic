pub use toml_config_derive::*;

pub trait TomlConfig {
    fn to_string(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Comment for TestConfig
    #[derive(TomlConfig, Default)]
    struct TestConfig {
        /// Comment for a
        a: String,
        /// Comment for b
        b: usize,
        /// Comment for c
        c: usize,
    }

    #[test]
    fn test() {
        let config = TestConfig::default();

        println!("{}", config.to_string());

        assert!(false);
    }
}