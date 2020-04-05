# TODO

* Configuration, cli
* Tests
* Optimize extract_response_peers
* Clean connections: `state.connections.shrink_to_fit()`?

## Don't do

* Other hash algorithms: seemingly not worthwhile
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help