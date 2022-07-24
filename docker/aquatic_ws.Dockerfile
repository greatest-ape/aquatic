# syntax=docker/dockerfile:1

# aquatic_ws
#
# WORK IN PROGRESS: currently has issues spawning worker threads, possibly
# related to https://github.com/DataDog/glommio/issues/547
#
# Customize by setting CONFIG_FILE_CONTENTS and
# ACCESS_LIST_CONTENTS environment variables.
#
# If no changes are made to configuration, aquatic_ws is run:
# - on port 3000
# - without TLS
# - with http health checks enabled
# - only allowing announces for hashes in access list, e.g., contained
#   in ACCESS_LIST_CONTENTS env var
#
# Run from root directory of aquatic repository with:
# $ DOCKER_BUILDKIT=1 docker build -t aquatic-ws -f docker/aquatic_ws.Dockerfile .
# $ docker run -it --ulimit memlock=65536:65536 -p 0.0.0.0:3000:3000 --name aquatic-ws aquatic-ws
#
# Pass --network="host" to run command for much better performance.

FROM rust:latest AS builder

WORKDIR /usr/src/aquatic

COPY . .

RUN . ./scripts/env-native-cpu-without-avx-512 && cargo build --release -p aquatic_ws

FROM debian:stable-slim

ENV CONFIG_FILE_CONTENTS "\
    log_level = 'info'\n\
    [network]\n\
    enable_http_health_checks = true\n\
    [access_list]\n\
    mode = 'allow'\n\
    "
ENV ACCESS_LIST_CONTENTS "0f0f0f0f0f1f1f1f1f1f2f2f2f2f2f3f3f3f3f3f"

WORKDIR /root/

COPY --from=builder /usr/src/aquatic/target/release/aquatic_ws ./

# Create entry point script for setting config and access
# list file contents at runtime
COPY <<-"EOT" ./entrypoint.sh
#!/bin/bash
echo -e "$CONFIG_FILE_CONTENTS" > ./config.toml
echo -e "$ACCESS_LIST_CONTENTS" > ./access-list.txt
exec ./aquatic_ws -c ./config.toml "$@"
EOT

RUN chmod +x ./entrypoint.sh

ENTRYPOINT ["./entrypoint.sh"]
