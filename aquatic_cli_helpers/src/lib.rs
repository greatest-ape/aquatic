use std::fs::File;
use std::io::Read;

use anyhow::Context;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use simplelog::{ConfigBuilder, LevelFilter, TermLogger, TerminalMode};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
        Self::Error
    }
}

pub trait Config: Default + Serialize + DeserializeOwned {
    fn get_log_level(&self) -> Option<LogLevel> {
        None
    }
}

#[derive(Debug, Default)]
pub struct Options {
    config_file: Option<String>,
    print_config: bool,
}

impl Options {
    pub fn parse_args<I>(mut arg_iter: I) -> Result<Options, Option<String>>
    where
        I: Iterator<Item = String>,
    {
        let mut options = Options::default();

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
                    "-h" | "--help" => {
                        return Err(None);
                    }
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
    // Function that takes config file and runs application
    app_fn: fn(T) -> anyhow::Result<()>,
    opts: Option<Options>,
) where
    T: Config,
{
    ::std::process::exit(match run_inner(app_title, app_fn, opts) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("Error: {:#}", err);

            1
        }
    })
}

fn run_inner<T>(
    app_title: &str,
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

    if options.print_config {
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
    println!("    -p, --print-config    Print default config");
    println!("    -h, --help            Print this help message");

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
    T: Default + Serialize,
{
    toml::to_string_pretty(&T::default()).expect("Could not serialize default config to toml")
}

fn start_logger(log_level: LogLevel) -> ::anyhow::Result<()> {
    let level_filter = match log_level {
        LogLevel::Off => LevelFilter::Off,
        LogLevel::Error => LevelFilter::Error,
        LogLevel::Warn => LevelFilter::Warn,
        LogLevel::Info => LevelFilter::Info,
        LogLevel::Debug => LevelFilter::Debug,
        LogLevel::Trace => LevelFilter::Trace,
    };

    // Note: logger doesn't seem to pick up thread names. Not a huge loss.
    let simplelog_config = ConfigBuilder::new()
        .set_time_to_local(true)
        .set_location_level(LevelFilter::Off)
        .build();

    TermLogger::init(level_filter, simplelog_config, TerminalMode::Stderr)
        .context("Couldn't initialize logger")?;

    Ok(())
}
