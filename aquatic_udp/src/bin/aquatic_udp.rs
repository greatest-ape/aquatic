use aquatic_udp;
use cli_helpers;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    cli_helpers::run_app_with_cli_and_config::<aquatic_udp::config::Config>(
        "aquatic: udp bittorrent tracker",
        aquatic_udp::run,
    )
}