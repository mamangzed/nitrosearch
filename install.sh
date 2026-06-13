#!/usr/bin/env bash

set -e

echo "========================================"
echo " NitroSearch Development Environment"
echo " Ubuntu 22.04"
echo "========================================"

echo "[1/8] Updating system..."

sudo apt update
sudo apt upgrade -y

echo "[2/8] Installing build dependencies..."

sudo apt install -y \
build-essential \
pkg-config \
curl \
wget \
git \
cmake \
clang \
llvm \
lld \
gdb \
make \
unzip \
zip \
jq \
htop \
tree \
libssl-dev \
libzstd-dev \
sqlite3 \
libsqlite3-dev \
protobuf-compiler \
libsnappy-dev \
liblz4-dev \
libbz2-dev

echo "[3/8] Installing Rust..."

if ! command -v rustc >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi

source "$HOME/.cargo/env"

echo "[4/8] Updating Rust toolchain..."

rustup update
rustup default stable

echo "[5/8] Installing Rust components..."

rustup component add \
rustfmt \
clippy \
rust-analyzer

echo "[6/8] Installing cargo utilities..."

cargo install cargo-watch || true
cargo install cargo-edit || true
cargo install cargo-audit || true
cargo install cargo-deny || true
cargo install cargo-nextest || true
cargo install cargo-expand || true
cargo install cargo-outdated || true
cargo install cargo-bloat || true
cargo install cargo-generate || true

echo "[7/8] Configuring fast linker..."

mkdir -p ~/.cargo

cat > ~/.cargo/config.toml << 'EOF'
[target.x86_64-unknown-linux-gnu]
linker = "clang"

rustflags = [
  "-C",
  "link-arg=-fuse-ld=lld"
]
EOF

echo "[8/8] Creating NitroSearch workspace..."

mkdir -p ~/nitrosearch/crates

cd ~/nitrosearch

cargo new crates/nitro-core --lib --vcs none
cargo new crates/nitro-index --lib --vcs none
cargo new crates/nitro-storage --lib --vcs none
cargo new crates/nitro-query --lib --vcs none
cargo new crates/nitro-ranking --lib --vcs none
cargo new crates/nitro-api --bin --vcs none
cargo new crates/nitro-cli --bin --vcs none

cat > Cargo.toml << 'EOF'
[workspace]
resolver = "2"

members = [
  "crates/nitro-core",
  "crates/nitro-index",
  "crates/nitro-storage",
  "crates/nitro-query",
  "crates/nitro-ranking",
  "crates/nitro-api",
  "crates/nitro-cli"
]
EOF

echo ""
echo "========================================"
echo " Installation Complete"
echo "========================================"
echo ""

rustc --version
cargo --version

echo ""
echo "Workspace:"
echo "~/nitrosearch"
echo ""