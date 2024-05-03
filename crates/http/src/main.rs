use aquatic_common::cli::run_app_with_cli_and_config;
use aquatic_http::config::Config;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    run_app_with_cli_and_config::<Config>(
        aquatic_http::APP_NAME,
        aquatic_http::APP_VERSION,
        aquatic_http::run,
        None,
    )
}
