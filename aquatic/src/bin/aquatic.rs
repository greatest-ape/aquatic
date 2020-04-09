use aquatic;
use cli_helpers;


#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;


fn main(){
    cli_helpers::run_with_cli_and_config(
        "aquatic: udp bittorrent tracker",
        aquatic::run,
    )
}