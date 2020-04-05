# TODO

* Configuration, cli
* Tests
* Benchmarks
    * Pareto distribution parameters OK?
* Clean connections: `state.connections.shrink_to_fit()`?


## Don't do

* Other hash algorithms: seemingly not worthwhile
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help