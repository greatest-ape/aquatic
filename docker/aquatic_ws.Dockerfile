# aquatic_ws
#
# WORK IN PROGRESS
#
# Customize by setting CONFIG_FILE_CONTENTS and
# ACCESS_LIST_CONTENTS environment variables.
#
# If no changes are made to configuration, aquatic_ws is run:
# - on port 3000
# - without TLS
# - with no info hash access control
# - with http health checks enabled
#
# Run from root directory of repository with:
# $ docker build -t aquatic-ws -f docker/aquatic_ws.Dockerfile .
# $ docker run -it --ulimit memlock=65536:65536 -p 0.0.0.0:3000:3000 --name aquatic-ws aquatic-ws

FROM rust:latest AS builder

WORKDIR /usr/src/aquatic

COPY . .

RUN . ./scripts/env-native-cpu-without-avx-512 && cargo build --release -p aquatic_ws

FROM debian:stable-slim

ENV CONFIG_FILE_CONTENTS "\
    log_level = 'info'\n\
    [network]\n\
    enable_http_health_checks = true\n\
    "
ENV ACCESS_LIST_CONTENTS ""

WORKDIR /root/

COPY --from=builder /usr/src/aquatic/target/release/aquatic_ws ./

# Enable setting config and access list file contents at runtime
RUN echo "#!/bin/sh\necho \"\$CONFIG_FILE_CONTENTS\" > ./config.toml\necho \"\$ACCESS_LIST_CONTENTS\" > ./access-list.txt\n./aquatic_ws -P -c ./config.toml" > entrypoint.sh && chmod +x entrypoint.sh

ENTRYPOINT ["./entrypoint.sh"]
