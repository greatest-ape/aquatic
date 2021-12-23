
#[cfg(test)]
mod tests {
    use toml_config::*;

    #[derive(TomlConfig, Default)]
    struct TestConfigInnerA {
        /// Comment for a
        a: String,
        /// Comment for b
        b: usize,
        /// Comment for c
        c: usize,
        // Comment for 
    }

    /// Comment for TestConfig
    #[derive(TomlConfig, Default)]
    struct TestConfig {
        /// Comment for a that stretches over
        /// multiple lines
        a: String,
        /// Comment for b
        b: usize,
        /// Comment for c
        c: bool,
        /// Comment for TestConfigInnerA
        inner_a: TestConfigInnerA,
    }

    #[test]
    fn test() {
        let config = TestConfig::default();

        println!("{}", TomlConfig::to_string(&config));

        assert!(false);
    }
}