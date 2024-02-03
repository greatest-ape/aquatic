use std::fs::File;
use std::io::Read;

use anyhow::Context;
use aquatic_toml_config::TomlConfig;
use git_testament::{git_testament, CommitKind};
use log::LevelFilter;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use simplelog::{ColorChoice, TermLogger, TerminalMode, ThreadLogMode};

/// Log level. Available values are off, error, warn, info, debug and trace.
#[derive(Debug, Clone, Copy, PartialEq, TomlConfig, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Warn
    }
}

pub trait Config: Default + TomlConfig + DeserializeOwned + std::fmt::Debug {
    fn get_log_level(&self) -> Option<LogLevel> {
        None
    }
}

#[derive(Debug, Default)]
pub struct Options {
    config_file: Option<String>,
    print_config: bool,
    print_parsed_config: bool,
    print_version: bool,
}

impl Options {
    pub fn parse_args<I>(mut arg_iter: I) -> Result<Options, Option<String>>
    where
        I: Iterator<Item = String>,
    {
        let mut options = Options::default();

        #[allow(clippy::while_let_loop)] // False positive
        loop {
            if let Some(arg) = arg_iter.next() {
                match arg.as_str() {
                    "-c" | "--config-file" => {
                        if let Some(path) = arg_iter.next() {
                            options.config_file = Some(path);
                        } else {
                            return Err(Some("No config file path given".to_string()));
                        }
                    }
                    "-p" | "--print-config" => {
                        options.print_config = true;
                    }
                    "-P" => {
                        options.print_parsed_config = true;
                    }
                    "-v" | "--version" => {
                        options.print_version = true;
                    }
                    "-h" | "--help" => {
                        return Err(None);
                    }
                    "" => (),
                    _ => {
                        return Err(Some("Unrecognized argument".to_string()));
                    }
                }
            } else {
                break;
            }
        }

        Ok(options)
    }
}

pub fn run_app_with_cli_and_config<T>(
    app_title: &str,
    crate_version: &str,
    // Function that takes config file and runs application
    app_fn: fn(T) -> anyhow::Result<()>,
    opts: Option<Options>,
) where
    T: Config,
{
    ::std::process::exit(match run_inner(app_title, crate_version, app_fn, opts) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("Error: {:#}", err);

            1
        }
    })
}

fn run_inner<T>(
    app_title: &str,
    crate_version: &str,
    // Function that takes config file and runs application
    app_fn: fn(T) -> anyhow::Result<()>,
    // Possibly preparsed options
    options: Option<Options>,
) -> anyhow::Result<()>
where
    T: Config,
{
    let options = if let Some(options) = options {
        options
    } else {
        let mut arg_iter = ::std::env::args();

        let app_path = arg_iter.next().unwrap();

        match Options::parse_args(arg_iter) {
            Ok(options) => options,
            Err(opt_err) => {
                let gen_info = || format!("{}\n\nUsage: {} [OPTIONS]", app_title, app_path);

                print_help(gen_info, opt_err);

                return Ok(());
            }
        }
    };

    if options.print_version {
        let commit_info = get_commit_info();

        println!("{}{}", crate_version, commit_info);

        Ok(())
    } else if options.print_config {
        print!("{}", default_config_as_toml::<T>());

        Ok(())
    } else {
        let config = if let Some(path) = options.config_file {
            config_from_toml_file(path)?
        } else {
            T::default()
        };

        if let Some(log_level) = config.get_log_level() {
            start_logger(log_level)?;
        }

        if options.print_parsed_config {
            println!("Running with configuration: {:#?}", config);
        }

        app_fn(config)
    }
}

pub fn print_help<F>(info_generator: F, opt_error: Option<String>)
where
    F: FnOnce() -> String,
{
    println!("{}", info_generator());

    println!("\nOptions:");
    println!("    -c, --config-file     Load config from this path");
    println!("    -h, --help            Print this help message");
    println!("    -p, --print-config    Print default config");
    println!("    -P                    Print parsed config");
    println!("    -v, --version         Print version information");

    if let Some(error) = opt_error {
        println!("\nError: {}.", error);
    }
}

fn config_from_toml_file<T>(path: String) -> anyhow::Result<T>
where
    T: DeserializeOwned,
{
    let mut file = File::open(path.clone())
        .with_context(|| format!("Couldn't open config file {}", path.clone()))?;

    let mut data = String::new();

    file.read_to_string(&mut data)
        .with_context(|| format!("Couldn't read config file {}", path.clone()))?;

    toml::from_str(&data).with_context(|| format!("Couldn't parse config file {}", path.clone()))
}

fn default_config_as_toml<T>() -> String
where
    T: Default + TomlConfig,
{
    <T as TomlConfig>::default_to_string()
}

fn start_logger(log_level: LogLevel) -> ::anyhow::Result<()> {
    let mut builder = simplelog::ConfigBuilder::new();

    builder
        .set_thread_mode(ThreadLogMode::Both)
        .set_thread_level(LevelFilter::Error)
        .set_target_level(LevelFilter::Error)
        .set_location_level(LevelFilter::Off);

    let config = match builder.set_time_offset_to_local() {
        Ok(builder) => builder.build(),
        Err(builder) => builder.build(),
    };

    let level_filter = match log_level {
        LogLevel::Off => LevelFilter::Off,
        LogLevel::Error => LevelFilter::Error,
        LogLevel::Warn => LevelFilter::Warn,
        LogLevel::Info => LevelFilter::Info,
        LogLevel::Debug => LevelFilter::Debug,
        LogLevel::Trace => LevelFilter::Trace,
    };

    TermLogger::init(
        level_filter,
        config,
        TerminalMode::Stderr,
        ColorChoice::Auto,
    )
    .context("Couldn't initialize logger")?;

    Ok(())
}

fn get_commit_info() -> String {
    git_testament!(TESTAMENT);

    match TESTAMENT.commit {
        CommitKind::NoTags(hash, date) => {
            format!(" ({} - {})", first_8_chars(hash), date)
        }
        CommitKind::FromTag(_tag, hash, date, _tag_distance) => {
            format!(" ({} - {})", first_8_chars(hash), date)
        }
        _ => String::new(),
    }
}

fn first_8_chars(input: &str) -> String {
    input.chars().take(8).collect()
}
