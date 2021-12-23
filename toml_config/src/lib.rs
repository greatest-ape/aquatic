use std::path::PathBuf;

pub use toml_config_derive::TomlConfig;

pub trait TomlConfig: Default {
    fn to_string_with_field_name(&self, field_name: String) -> String;
    fn to_string(&self) -> String {
        unimplemented!()
    }
}

impl TomlConfig for usize {
    fn to_string_with_field_name(&self, field_name: String) -> String {
        let value = toml::ser::to_string(self).unwrap();

        format!("{} = {}\n", field_name, value)
    }
}

impl TomlConfig for String {
    fn to_string_with_field_name(&self, field_name: String) -> String {
        let value = toml::ser::to_string(self).unwrap();

        format!("{} = {}\n", field_name, value)
    }
}
