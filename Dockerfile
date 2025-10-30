# ---------- build stage ----------
    FROM rust:1.81-bullseye AS builder
    WORKDIR /app
    
    # Кэш зависимостей
    COPY Cargo.toml Cargo.lock ./
    RUN mkdir -p src && echo "fn main(){}" > src/main.rs
    RUN cargo fetch
    
    # Полная сборка с rustls
    COPY . .
    RUN cargo build --release --no-default-features --features tls-rustls
    
    # ---------- runtime stage ----------
    FROM debian:bullseye-slim
    RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
    WORKDIR /app
    
    # Бинарник
    COPY --from=builder /app/target/release/openai-proxy /usr/local/bin/openai-proxy
    
    # Непривилегированный пользователь
    RUN useradd -r -s /usr/sbin/nologin proxyuser
    USER proxyuser
    
    ENV RUST_LOG=info
    EXPOSE 8080
    CMD ["/usr/local/bin/openai-proxy"]
    