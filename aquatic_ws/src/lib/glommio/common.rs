use std::sync::Arc;

use aquatic_common::access_list::AccessListArcSwap;

pub type TlsConfig = futures_rustls::rustls::ServerConfig;

#[derive(Default, Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
}
