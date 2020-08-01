use std::fs::File;
use std::io::Read;

use anyhow::Context;
use gumdrop::Options;
use serde::{Serialize, de::DeserializeOwned};


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
) where T: Default + Serialize + DeserializeOwned {
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
) -> anyhow::Result<()> where T: Default + Serialize + DeserializeOwned {
    let args: Vec<String> = ::std::env::args().collect();

    let opts = AppOptions::parse_args_default(&args[1..])?;

    if opts.help_requested(){
        print_help(title, None);

        Ok(())
    } else if opts.print_config {
        print!("{}", default_config_as_toml::<T>());

        Ok(())
    } else if let Some(config_file) = opts.config_file {
        let config = config_from_toml_file(config_file)?;

        app_fn(config)
    } else {
        app_fn(T::default())
    }
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


fn print_help(title: &str, opt_error: Option<anyhow::Error>){
    println!("{}", title);

    if let Some(error) = opt_error {
        println!("\nError: {:#}.", error);
    }

    println!("\n{}", AppOptions::usage());
}
