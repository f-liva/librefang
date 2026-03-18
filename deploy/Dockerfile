# syntax=docker/dockerfile:1
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    libssl-dev \
    perl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Build dependencies first (cached unless Cargo.toml/Cargo.lock change)
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY agents ./agents
COPY catalog ./catalog
COPY packages ./packages
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin librefang \
    && cp target/release/librefang /usr/local/bin/librefang

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl git ffmpeg python3 python3-pip chromium gosu sudo procps build-essential jq && rm -rf /var/lib/apt/lists/*
RUN ln -s /usr/bin/python3 /usr/bin/python
RUN curl -LsSf https://astral.sh/uv/install.sh | sh
RUN (type -p wget >/dev/null || (apt-get update && apt-get install -y wget)) && \
    mkdir -p -m 755 /etc/apt/keyrings && \
    out=$(mktemp) && wget -qO "$out" https://cli.github.com/packages/githubcli-archive-keyring.gpg && \
    cat "$out" | tee /etc/apt/keyrings/githubcli-archive-keyring.gpg > /dev/null && \
    chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg && \
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null && \
    apt-get update && apt-get install -y gh && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && \
    apt-get install -y nodejs && \
    npm install -g @anthropic-ai/claude-code @qwen-code/qwen-code && \
    rm -rf /var/lib/apt/lists/*
RUN useradd -m -s /bin/bash librefang && echo "librefang ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/librefang
USER librefang
RUN NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
RUN eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)" && brew install steipete/tap/gogcli
USER root
RUN echo 'eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"' >> /home/librefang/.bashrc && \
    echo 'eval "$(/home/linuxbrew/.linuxbrew/bin/brew shellenv)"' >> /root/.bashrc && \
    echo 'export PATH="/data/npm-global/bin:$PATH"' >> /home/librefang/.bashrc && \
    echo 'export PATH="/data/npm-global/bin:$PATH"' >> /root/.bashrc
COPY --from=builder /usr/local/bin/librefang /usr/local/bin/
COPY --from=builder /build/agents /opt/librefang/agents
COPY --from=builder /build/packages /opt/librefang/packages
RUN mkdir -p /data && chown librefang:librefang /data
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
EXPOSE 4545
VOLUME /data
ENV LIBREFANG_HOME=/data
ENTRYPOINT ["entrypoint.sh"]
