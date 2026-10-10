# outliers-sim server image: the static binary plus git (projects are git
# repositories) on a minimal Alpine base.
#
#   docker build -t outliers-sim .
#   docker run -d -p 8080:8080 -v outliers-sim-data:/data outliers-sim
#
# /data/projects holds the projects (an empty volume gets the example
# projects); /data/runs holds results. See docs/deploy.md.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
# Alpine's Rust targets musl, so this is a static binary.
RUN cargo build --release --locked -p outliers-sim \
 && cp target/release/outliers-sim /outliers-sim

FROM alpine:3
RUN apk add --no-cache git \
 && mkdir -p /data \
 && chown 65532:65532 /data
COPY --from=build /outliers-sim /usr/local/bin/outliers-sim
USER 65532:65532
ENV HOME=/data
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["outliers-sim"]
CMD ["serve", "--data", "/data", "--host", "0.0.0.0", "--port", "8080"]
