# Not used by Github action, but can be used to run test locally:
#   1. docker build -t aquatic ./path/to/Dockerfile
#   2. docker run aquatic
#   3. On failure, run `docker rmi aquatic -f` and go back to step 1

FROM rust:bullseye

COPY entrypoint.sh /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]