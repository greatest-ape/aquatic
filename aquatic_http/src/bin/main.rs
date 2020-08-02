use aquatic_cli_helpers::run_app_with_cli_and_config;
use aquatic_http::config::Config;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    run_app_with_cli_and_config::<Config>(
        "aquatic: BitTorrent (HTTP/TLS) tracker",
        aquatic_http::run
    )
}