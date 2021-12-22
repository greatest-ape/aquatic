pub use toml_config_derive::*;

pub trait TomlConfig: Default {
    fn to_string(&self) -> String;
}
