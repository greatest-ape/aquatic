# TODO

* Configuration, cli
* Tests
* extract_response_peers
    * Cleaner code
    * Stack-allocated vector?
* Benchmarks
    * Move to own crate (aquatic_bench)
    * Better black_box (or make sure to consume data)
    * Show standard deviation?
    * Send in connect reponse ids to other functions as integration test
* target-cpu=native

## Don't do

* Other hash algorithms: seemingly not worthwhile (might be with AVX though)
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help