use aquatic_udp;
use aquatic_cli_helpers;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    aquatic_cli_helpers::run_app_with_cli_and_config::<aquatic_udp::config::Config>(
        "aquatic: udp bittorrent tracker",
        aquatic_udp::run,
    )
}