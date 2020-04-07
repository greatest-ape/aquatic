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
    * Save last results, check if difference is significant?
    * ProgressBar: `[{elapsed_precise}]` and eta_precise?
    * Test server over udp socket instead?
* Performance
    * cpu-target=native good?
    * mialloc good?
* bittorrent_udp
    * ParseError enum maybe, with `Option<TransactionId>`
    * Avoid allocating in conversion to bytes, send in a mutable buffer
      instead, which is reused over requests
    * Avoid heap allocation in general if it can be avoided?
    * quickcheck tests for conversions
    * other unit tests?

## Don't do

* Other HashMap hashers (such as SeaHash): seemingly not worthwhile (might be
  with AVX though)
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help
* Config behind Arc in state: it is likely better to be able to pass it around
  without state