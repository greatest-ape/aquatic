use aquatic_cli_helpers::run_app_with_cli_and_config;
use aquatic_http::config::Config;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    run_app_with_cli_and_config::<Config>(aquatic_http::APP_NAME, aquatic_http::run, None)
}
