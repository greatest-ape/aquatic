#!/bin/bash
# Run file transfer CI test locally

set -e

docker build -t aquatic -f ./docker/ci.Dockerfile .
docker run aquatic
docker rmi aquatic -f