use std::fs::File;
use std::io::Read;

use anyhow::Context;
use gumdrop::Options;
use serde::{Serialize, Deserialize, de::DeserializeOwned};
use simplelog::{ConfigBuilder, LevelFilter, TermLogger, TerminalMode};


#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace
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


#[derive(Debug, Options)]
struct AppOptions {
    #[options(help = "run with config file", short = "c", meta = "PATH")]
    config_file: Option<String>,
    #[options(help = "print default config file", short = "p")]
    print_config: bool,
    #[options(help = "print help message")]
    help: bool,
}


pub fn run_app_with_cli_and_config<T>(
    title: &str,
    // Function that takes config file and runs application
    app_fn: fn(T) -> anyhow::Result<()>,
) where T: Config {
    ::std::process::exit(match run_inner(title, app_fn) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("Error: {:#}", err);

            1
        },
    })
}


fn run_inner<T>(
    title: &str,
    // Function that takes config file and runs application
    app_fn: fn(T) -> anyhow::Result<()>,
) -> anyhow::Result<()> where T: Config {
    let args: Vec<String> = ::std::env::args().collect();

    let opts = AppOptions::parse_args_default(&args[1..])?;

    if opts.help_requested(){
        print_help(title, None);

        Ok(())
    } else if opts.print_config {
        print!("{}", default_config_as_toml::<T>());

        Ok(())
    } else if let Some(config_file) = opts.config_file {
        let config: T = config_from_toml_file(config_file)?;

        if let Some(log_level) = config.get_log_level(){
            start_logger(log_level)?;
        }

        app_fn(config)
    } else {
        let config = T::default();

        if let Some(log_level) = config.get_log_level(){
            start_logger(log_level)?;
        }

        app_fn(config)
    }
}


fn print_help(title: &str, opt_error: Option<anyhow::Error>){
    println!("{}", title);

    if let Some(error) = opt_error {
        println!("\nError: {:#}.", error);
    }

    println!("\n{}", AppOptions::usage());
}


fn config_from_toml_file<T>(path: String) -> anyhow::Result<T>
    where T: DeserializeOwned
{
    let mut file = File::open(path.clone()).with_context(||
        format!("Couldn't open config file {}", path.clone())
    )?;

    let mut data = String::new();

    file.read_to_string(&mut data).with_context(||
        format!("Couldn't read config file {}", path.clone())
    )?;

    toml::from_str(&data).with_context(||
        format!("Couldn't parse config file {}", path.clone())
    )
}


fn default_config_as_toml<T>() -> String
    where T: Default + Serialize
{
    toml::to_string_pretty(&T::default())
        .expect("Could not serialize default config to toml")
}


fn start_logger(log_level: LogLevel) -> ::anyhow::Result<()> {
    let level_filter = match log_level{
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

    TermLogger::init(
        level_filter,
        simplelog_config,
        TerminalMode::Stderr
    ).context("Couldn't initialize logger")?;

    Ok(())
}