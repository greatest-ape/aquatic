use std::borrow::Borrow;
use std::cell::RefCell;
use std::rc::Rc;

use futures_lite::AsyncBufReadExt;
use glommio::io::{BufferedFile, StreamReaderBuilder};
use glommio::prelude::*;

use crate::common::*;
use crate::config::Config;

pub async fn update_access_list<C: Borrow<Config>>(
    config: C,
    access_list: Rc<RefCell<AccessList>>,
) {
    if config.borrow().access_list.mode.is_on() {
        match BufferedFile::open(&config.borrow().access_list.path).await {
            Ok(file) => {
                let mut reader = StreamReaderBuilder::new(file).build();
                let mut new_access_list = AccessList::default();

                loop {
                    let mut buf = String::with_capacity(42);

                    match reader.read_line(&mut buf).await {
                        Ok(_) => {
                            if let Err(err) = new_access_list.insert_from_line(&buf) {
                                ::log::error!(
                                    "Couln't parse access list line '{}': {:?}",
                                    buf,
                                    err
                                );
                            }
                        }
                        Err(err) => {
                            ::log::error!("Couln't read access list line {:?}", err);

                            break;
                        }
                    }

                    yield_if_needed().await;
                }

                *access_list.borrow_mut() = new_access_list;
            }
            Err(err) => {
                ::log::error!("Couldn't open access list file: {:?}", err)
            }
        };
    }
}
