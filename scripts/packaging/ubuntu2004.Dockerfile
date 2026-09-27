# gx_package.py prepares the three-file build context for this image.
FROM ubuntu:20.04
ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup \
    PATH=/usr/local/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git lsb-release pkg-config perl \
    && rm -rf /var/lib/apt/lists/*
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --profile minimal --default-toolchain stable --no-modify-path \
    && rm /tmp/rustup.sh
WORKDIR /gx-deps
COPY get-deps ./
COPY check-rust-version.sh ./ci/
RUN apt-get update && bash ./get-deps && rm -rf /var/lib/apt/lists/*
# Bind-mounted repositories can have a different owner from the container user.
RUN git config --system safe.directory '*'
