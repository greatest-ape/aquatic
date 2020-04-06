# TODO

* Configuration, cli
* Tests
* extract_response_peers
    * Cleaner code
    * Stack-allocated vector?
* Benchmarks
    * Seperate setup so actual benchmarks can be run after each other,
      enabling better profiling
    * Show standard deviation?
    * Send in connect reponse ids to other functions as integration test

## Don't do

* Other hash algorithms: seemingly not worthwhile
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help