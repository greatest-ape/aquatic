
#[cfg(test)]
mod tests {
    use toml_config::*;

    /// Comment for TestConfig
    #[derive(TomlConfig2, Default)]
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