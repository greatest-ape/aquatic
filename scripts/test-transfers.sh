#!/bin/bash

docker build -t aquatic-test-transfers .github/actions/test-transfers
docker run -it aquatic-test-transfers bash