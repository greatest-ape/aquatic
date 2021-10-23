use std::cell::RefCell;
use std::rc::Rc;

use futures_lite::AsyncBufReadExt;
use glommio::io::{BufferedFile, StreamReaderBuilder};
use glommio::prelude::*;

use crate::common::*;
use crate::config::Config;

pub async fn update_access_list(config: Config, access_list: Rc<RefCell<AccessList>>) {
    if config.access_list.mode.is_on() {
        match BufferedFile::open(config.access_list.path).await {
            Ok(file) => {
                let mut reader = StreamReaderBuilder::new(file).build();

                loop {
                    let mut buf = String::with_capacity(42);

                    match reader.read_line(&mut buf).await {
                        Ok(_) => {
                            if let Err(err) = access_list.borrow_mut().insert_from_line(&buf) {
                                ::log::error!("Couln't parse access list line '{}': {:?}", buf, err);
                            }
                        }
                        Err(err) => {
                            ::log::error!("Couln't read access list line {:?}", err);

                            break;
                        }
                    }

                    yield_if_needed().await;
                }
            },
            Err(err) => {
                ::log::error!("Couldn't open access list file: {:?}", err)
            }
        };
    }
}
