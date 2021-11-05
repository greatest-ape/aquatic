use std::sync::Arc;

use aquatic_common::access_list::AccessListArcSwap;

#[derive(Default, Clone)]
pub struct State {
    pub access_list: Arc<AccessListArcSwap>,
}
