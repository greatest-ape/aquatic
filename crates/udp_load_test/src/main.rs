use aquatic_udp_load_test::config::Config;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub fn main() {
    aquatic_common::cli::run_app_with_cli_and_config::<Config>(
        "aquatic_udp_load_test: BitTorrent load tester",
        env!("CARGO_PKG_VERSION"),
        aquatic_udp_load_test::run,
        None,
    )
}
