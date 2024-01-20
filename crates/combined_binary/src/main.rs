use aquatic_common::cli::{print_help, run_app_with_cli_and_config, Options};
use aquatic_http::config::Config as HttpConfig;
use aquatic_udp::config::Config as UdpConfig;
use aquatic_ws::config::Config as WsConfig;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

const APP_NAME: &str = "aquatic: BitTorrent tracker";

fn main() {
    ::std::process::exit(match run() {
        Ok(()) => 0,
        Err(None) => {
            print_help(gen_info, None);

            0
        }
        Err(opt_err @ Some(_)) => {
            print_help(gen_info, opt_err);

            1
        }
    })
}

fn run() -> Result<(), Option<String>> {
    let mut arg_iter = ::std::env::args().skip(1);

    let protocol = if let Some(protocol) = arg_iter.next() {
        protocol
    } else {
        return Err(None);
    };

    let options = match Options::parse_args(arg_iter) {
        Ok(options) => options,
        Err(opt_err) => {
            return Err(opt_err);
        }
    };

    match protocol.as_str() {
        "udp" => run_app_with_cli_and_config::<UdpConfig>(
            aquatic_udp::APP_NAME,
            aquatic_udp::APP_VERSION,
            aquatic_udp::run,
            Some(options),
        ),
        "http" => run_app_with_cli_and_config::<HttpConfig>(
            aquatic_http::APP_NAME,
            aquatic_http::APP_VERSION,
            aquatic_http::run,
            Some(options),
        ),
        "ws" => run_app_with_cli_and_config::<WsConfig>(
            aquatic_ws::APP_NAME,
            aquatic_ws::APP_VERSION,
            aquatic_ws::run,
            Some(options),
        ),
        arg => {
            let opt_err = if arg == "-h" || arg == "--help" {
                None
            } else if arg.starts_with('-') {
                Some("First argument must be protocol".to_string())
            } else {
                Some("Invalid protocol".to_string())
            };

            return Err(opt_err);
        }
    }

    Ok(())
}

fn gen_info() -> String {
    let mut info = String::new();

    info.push_str(APP_NAME);

    let app_path = ::std::env::args().next().unwrap();
    info.push_str(&format!("\n\nUsage: {} PROTOCOL [OPTIONS]", app_path));
    info.push_str("\n\nAvailable protocols:");
    info.push_str("\n    udp                   BitTorrent over UDP");
    info.push_str("\n    http                  BitTorrent over HTTP");
    info.push_str("\n    ws                    WebTorrent");

    info
}
