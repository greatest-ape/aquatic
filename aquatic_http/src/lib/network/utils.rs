use std::time::Instant;

use anyhow::Context;
use mio::Token;

use super::*;


/// Don't bother with deregistering from Poll. In my understanding, this is
/// done automatically when the stream is dropped, as long as there are no
/// other references to the file descriptor, such as when it is accessed
/// in multiple threads.
pub fn remove_connection_if_exists(
    connections: &mut ConnectionMap,
    token: Token,
){
    if let Some(mut connection) = connections.remove(&token){
        // connection.close(); // FIXME
    }
}


// Close and remove inactive connections
pub fn remove_inactive_connections(
    connections: &mut ConnectionMap,
){
    let now = Instant::now();

    connections.retain(|_, connection| {
        if connection.valid_until.0 < now {
            // connection.close(); // FIXME

            false
        } else {
            true
        }
    });

    connections.shrink_to_fit();
}