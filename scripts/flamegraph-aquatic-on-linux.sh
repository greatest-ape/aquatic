#!/bin/bash

# Profile
perf record --call-graph=dwarf,16384 -e cpu-clock -F 997 target/release/aquatic

# Generate flamegraph (make sure nginx is installed for stdout path)
# Info: https://gist.github.com/dlaehnemann/df31787c41bd50c0fe223df07cf6eb89
perf script | stackcollapse-perf.pl | c++filt | flamegraph.pl > /var/www/html/flame.svg