use aquatic_common::access_list::{AccessListMode, AccessListQuery};
use histogram::Histogram;

use crate::common::*;
use crate::config::Config;

pub fn update_access_list(config: &Config, state: &State) {
    match config.access_list.mode {
        AccessListMode::White | AccessListMode::Black => {
            if let Err(err) = state.access_list.update_from_path(&config.access_list.path) {
                ::log::error!("Couldn't update access list: {:?}", err);
            }
        }
        AccessListMode::Off => {}
    }
}
