# syntax=docker/dockerfile:1

FROM --platform=$BUILDPLATFORM golang:1.23-alpine AS builder
ARG TARGETOS=linux
ARG TARGETARCH
WORKDIR /src

COPY go.mod ./
COPY relay.go relay_stream.go relay_ws.go ./
COPY public ./public

RUN CGO_ENABLED=0 GOOS=${TARGETOS} GOARCH=${TARGETARCH} \
    go build -trimpath -ldflags="-s -w" -o /out/naro-relay .

FROM scratch
LABEL org.opencontainers.image.title="Naro relay" \
      org.opencontainers.image.url="https://naro.chat" \
      org.opencontainers.image.licenses="MIT"

COPY --from=builder /out/naro-relay /naro-relay

ENV NARO_RELAY_ADDR=0.0.0.0:7319
EXPOSE 7319
USER 65532:65532

ENTRYPOINT ["/naro-relay"]
