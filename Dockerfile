FROM rust as builder
WORKDIR /app

# a trick to avoid downloading and building 
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

COPY . .
RUN cargo install --path .

FROM gcr.io/distroless/cc-debian12
WORKDIR /app
COPY --from=builder /app/target/release/red .
COPY --from=builder /app/config.toml .
COPY --from=builder /app/themes ./themes
COPY --from=builder /app/langs ./langs
COPY --from=builder /app/test ./test
ENV RED_HOME=/app
CMD ["./red"]

# docker build -t red:latest .
# docker run -it --rm red:latest