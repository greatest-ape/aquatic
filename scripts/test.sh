#!/bin/sh

# Not chosen for exact values, only to be larger than defaults
export QUICKCHECK_TESTS=2000
export QUICKCHECK_GENERATOR_SIZE=1000

cargo test
