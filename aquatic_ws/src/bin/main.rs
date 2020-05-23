use anyhow::Context;
use cli_helpers::run_app_with_cli_and_config;
use simplelog::{ConfigBuilder, LevelFilter, TermLogger, TerminalMode, ThreadLogMode};

use aquatic_ws::config::{Config, LogLevel};


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    run_app_with_cli_and_config::<Config>(
        "aquatic: webtorrent tracker",
        run
    )
}


fn run(config: Config) -> anyhow::Result<()> {
    let level_filter = match config.log_level {
        LogLevel::Off => LevelFilter::Off,
        LogLevel::Error => LevelFilter::Error,
        LogLevel::Warn => LevelFilter::Warn,
        LogLevel::Info => LevelFilter::Info,
        LogLevel::Debug => LevelFilter::Debug,
        LogLevel::Trace => LevelFilter::Trace,
    };

    let simplelog_config = ConfigBuilder::new()
        .set_time_to_local(true)
        .set_location_level(LevelFilter::Off)
        .set_thread_level(LevelFilter::Error)
        .set_thread_mode(ThreadLogMode::Both)
        .build();

    TermLogger::init(
        level_filter,
        simplelog_config,
        TerminalMode::Stdout
    ).context("Couldn't initialize logger")?;

    aquatic_ws::run(config)
}