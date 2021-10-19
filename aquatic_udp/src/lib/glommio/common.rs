use std::cell::RefCell;
use std::rc::Rc;

use futures_lite::{AsyncBufReadExt};
use glommio::io::{BufferedFile, StreamReaderBuilder};
use glommio::{prelude::*};

use crate::common::*;
use crate::config::Config;

pub async fn update_access_list(
    config: Config,
    access_list: Rc<RefCell<AccessList>>,
){
    if config.access_list.mode.is_on(){
        let access_list_file = BufferedFile::open(config.access_list.path).await.unwrap();

        let mut reader = StreamReaderBuilder::new(access_list_file).build();

        loop {
            let mut buf = String::with_capacity(42);

            match reader.read_line(&mut buf).await {
                Ok(_) => {
                    access_list.borrow_mut().insert_from_line(&buf);
                },
                Err(err) => {
                    break;
                }
            }

            yield_if_needed().await;
        }
    }
}
