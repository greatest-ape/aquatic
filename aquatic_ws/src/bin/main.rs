use aquatic_ws;
use cli_helpers;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    cli_helpers::run_app_with_cli_and_config::<aquatic_ws::config::Config>(
        "aquatic: webtorrent tracker",
        aquatic_ws::run,
    )
}