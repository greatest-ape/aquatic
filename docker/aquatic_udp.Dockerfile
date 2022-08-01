# syntax=docker/dockerfile:1

# aquatic_udp
#
# Customize by setting CONFIG_FILE_CONTENTS and
# ACCESS_LIST_CONTENTS environment variables.
#
# By default runs tracker on port 3000 without info hash access control.
#
# Run from repository root directory with:
# $ DOCKER_BUILDKIT=1 docker build -t aquatic-udp -f docker/aquatic_udp.Dockerfile .
# $ docker run -it -p 0.0.0.0:3000:3000/udp --name aquatic-udp aquatic-udp
#
# Pass --network="host" to run command for much better performance.

FROM rust:latest AS builder

WORKDIR /usr/src/aquatic

COPY . .

RUN . ./scripts/env-native-cpu-without-avx-512 && cargo build --release -p aquatic_udp

FROM debian:stable-slim

ENV CONFIG_FILE_CONTENTS "log_level = 'warn'"
ENV ACCESS_LIST_CONTENTS ""

WORKDIR /root/

COPY --from=builder /usr/src/aquatic/target/release/aquatic_udp ./

# Create entry point script for setting config and access
# list file contents at runtime
COPY <<-"EOT" ./entrypoint.sh
#!/bin/bash
echo -e "$CONFIG_FILE_CONTENTS" > ./config.toml
echo -e "$ACCESS_LIST_CONTENTS" > ./access-list.txt
exec ./aquatic_udp -c ./config.toml "$@"
EOT

RUN chmod +x ./entrypoint.sh

ENTRYPOINT ["./entrypoint.sh"]
