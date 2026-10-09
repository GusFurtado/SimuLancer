# outliers-sim server image: the static binary on an empty base image.
#
#   docker build -t outliers-sim .
#   docker run -d -p 8080:8080 -v outliers-sim-data:/data outliers-sim
#
# /data/models holds model and scenario files (the examples, to start with);
# /data/runs holds results. See docs/deploy.md.

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
# Alpine's Rust targets musl, so this is a static binary.
RUN cargo build --release --locked -p outliers-sim \
 && cp target/release/outliers-sim /outliers-sim
# Folders the server writes to, owned by the unprivileged user it runs as.
RUN mkdir -p /data/models /data/runs \
 && cp examples/*.yaml /data/models/ \
 && chown -R 65532:65532 /data

FROM scratch
COPY --from=build /outliers-sim /outliers-sim
COPY --from=build /data /data
USER 65532:65532
EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["/outliers-sim"]
CMD ["serve", "--models", "/data/models", "--runs", "/data/runs", "--host", "0.0.0.0", "--port", "8080"]
