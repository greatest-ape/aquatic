# aquatic_udp
#
# Customize by setting CONFIG_FILE_CONTENTS and
# ACCESS_LIST_CONTENTS environment variables.
#
# $ docker build -t aquatic-udp -f docker/aquatic_udp.Dockerfile .
# $ docker run -it -p 0.0.0.0:3000:3000/udp --name aquatic-udp aquatic-udp

FROM rust:latest AS builder

WORKDIR /usr/src/aquatic

COPY . .

RUN . ./scripts/env-native-cpu-without-avx-512 && cargo build --release -p aquatic_udp

FROM debian:stable-slim

ENV CONFIG_FILE_CONTENTS "log_level = 'warn'"
ENV ACCESS_LIST_CONTENTS ""

WORKDIR /root/

COPY --from=builder /usr/src/aquatic/target/release/aquatic_udp ./

# Setting config and access list file contents at runtime
RUN echo "#!/bin/sh\necho \"\$CONFIG_FILE_CONTENTS\" > ./config.toml\necho \"\$ACCESS_LIST_CONTENTS\" > ./access-list.txt\n./aquatic_udp -c ./config.toml" > entrypoint.sh && chmod +x entrypoint.sh

ENTRYPOINT ["./entrypoint.sh"]
