use aquatic_common::cli::run_app_with_cli_and_config;
use aquatic_http_private::config::Config;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    run_app_with_cli_and_config::<Config>(
        aquatic_http_private::APP_NAME,
        aquatic_http_private::APP_VERSION,
        aquatic_http_private::run,
        None,
    )
}
