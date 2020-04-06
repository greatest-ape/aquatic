# TODO

* Configuration, cli
* Tests
* extract_response_peers
    * Cleaner code
    * Stack-allocated vector?
* Benchmarks
    * num_rounds command line argument
    * Better black_box (or make sure to consume data)
    * Send in connect reponse ids to other functions as integration test
* Performance
    * cpu-target=native good?
    * mialloc good?

## Don't do

* Other hash algorithms: seemingly not worthwhile (might be with AVX though)
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help