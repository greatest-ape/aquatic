pub use toml_config_derive::TomlConfig;

macro_rules! impl_trait {
    ($ident:ident) => {
        impl Private for $ident {
            fn __to_string(&self, comment: Option<String>, field_name: String) -> String {
                let mut output = String::new();

                if let Some(comment) = comment {
                    output.push_str(&comment);
                }

                let value = toml::ser::to_string(self).unwrap();

                output.push_str(&format!("{} = {}\n", field_name, value));

                output
            }
        }
    };
}

pub trait TomlConfig: Default {
    fn default_to_string(&self) -> String;
}

pub mod __private {
    use std::path::PathBuf;

    pub trait Private: Default {
        fn __to_string(&self, comment: Option<String>, field_name: String) -> String;
    }

    impl_trait!(usize);
    impl_trait!(bool);
    impl_trait!(String);
    impl_trait!(PathBuf);
}
