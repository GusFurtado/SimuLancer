# SimuLancer server image: the static binary plus git (projects are git
# repositories) on a minimal Alpine base.
#
#   docker build -t simulancer .
#   docker run -d -p 8080:8080 -v simulancer-data:/data simulancer
#
# /data/projects holds the projects (an empty volume gets the example
# projects); /data/runs holds results. See docs/deploy.md.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
# Alpine's Rust targets musl, so this is a static binary.
RUN cargo build --release --locked -p simulancer \
 && cp target/release/simulancer /simulancer

FROM alpine:3
RUN apk add --no-cache git \
 && mkdir -p /data \
 && chown 65532:65532 /data
COPY --from=build /simulancer /usr/local/bin/simulancer
USER 65532:65532
ENV HOME=/data
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["simulancer"]
CMD ["serve", "--data", "/data", "--host", "0.0.0.0", "--port", "8080"]
