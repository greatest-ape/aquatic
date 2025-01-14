pub use aquatic_toml_config_derive::TomlConfig;
pub use toml;

/// Run this on your struct implementing TomlConfig to generate a
/// serialization/deserialization test for it.
#[macro_export]
macro_rules! gen_serialize_deserialize_test {
    ($ident:ident) => {
        #[test]
        fn test_cargo_toml_serialize_deserialize() {
            use ::aquatic_toml_config::TomlConfig;
            let serialized = $ident::default_to_string();
            let deserialized = ::aquatic_toml_config::toml::de::from_str(&serialized).unwrap();

            assert_eq!($ident::default(), deserialized);
        }
    };
}

/// Export structs to toml, converting Rust doc strings to comments.
///
/// Supports one level of nesting. Fields containing structs must come
/// after regular fields.
///
/// Usage:
/// ```
/// use aquatic_toml_config::TomlConfig;
///
/// #[derive(TomlConfig)]
/// struct SubConfig {
///     /// A
///     a: usize,
///     /// B
///     b: String,
/// }
///
/// impl Default for SubConfig {
///     fn default() -> Self {
///         Self {
///             a: 200,
///             b: "subconfig hello".into(),
///         }
///     }
/// }
///
/// #[derive(TomlConfig)]
/// struct Config {
///     /// A
///     a: usize,
///     /// B
///     b: String,
///     /// C
///     c: SubConfig,
/// }
///
/// impl Default for Config {
///     fn default() -> Self {
///         Self {
///             a: 100,
///             b: "hello".into(),
///             c: Default::default(),
///         }
///     }
/// }
///
/// let expected = "# A\na = 100\n# B\nb = \"hello\"\n\n# C\n[c]\n# A\na = 200\n# B\nb = \"subconfig hello\"\n";
///
/// assert_eq!(
///     Config::default_to_string(),
///     expected,
/// );
/// ```
pub trait TomlConfig: Default {
    fn default_to_string() -> String;
}

pub mod __private {
    use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
    use std::path::PathBuf;

    pub trait Private {
        fn __to_string(&self, comment: Option<String>, field_name: String) -> String;
    }

    macro_rules! impl_trait {
        ($ident:ident) => {
            impl Private for $ident {
                fn __to_string(&self, comment: Option<String>, field_name: String) -> String {
                    let mut output = String::new();

                    if let Some(comment) = comment {
                        output.push_str(&comment);
                    }

                    let value = crate::toml::ser::to_string(self).unwrap();

                    output.push_str(&format!("{} = {}\n", field_name, value));

                    output
                }
            }
        };
    }

    impl_trait!(isize);
    impl_trait!(i8);
    impl_trait!(i16);
    impl_trait!(i32);
    impl_trait!(i64);

    impl_trait!(usize);
    impl_trait!(u8);
    impl_trait!(u16);
    impl_trait!(u32);
    impl_trait!(u64);

    impl_trait!(f32);
    impl_trait!(f64);

    impl_trait!(bool);

    impl_trait!(String);

    impl_trait!(PathBuf);
    impl_trait!(SocketAddr);
    impl_trait!(SocketAddrV4);
    impl_trait!(SocketAddrV6);
}
