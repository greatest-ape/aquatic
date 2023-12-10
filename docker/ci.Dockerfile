# Can be used to run file transfer CI test locally. Usage:
#   1. docker build -t aquatic -f ./docker/ci.Dockerfile .
#   2. docker run aquatic
#   3. On failure, run `docker rmi aquatic -f` and go back to step 1

FROM rust:bullseye

RUN mkdir "/opt/aquatic"

ENV "GITHUB_WORKSPACE" "/opt/aquatic"

WORKDIR "/opt/aquatic"

COPY ./.github/actions/test-file-transfers/entrypoint.sh entrypoint.sh
COPY Cargo.toml Cargo.lock ./
COPY crates crates

ENTRYPOINT ["./entrypoint.sh"]