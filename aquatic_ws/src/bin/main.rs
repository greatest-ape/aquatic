use aquatic_cli_helpers::run_app_with_cli_and_config;
use aquatic_ws::config::Config;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    run_app_with_cli_and_config::<Config>(
        "aquatic: webtorrent tracker",
        aquatic_ws::run
    )
}