use aquatic_common::cli::run_app_with_cli_and_config;
use aquatic_ws::config::Config;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    run_app_with_cli_and_config::<Config>(
        aquatic_ws::APP_NAME,
        aquatic_ws::APP_VERSION,
        aquatic_ws::run,
        None,
    )
}
