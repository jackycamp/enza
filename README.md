# enza

A fast and light TUI diff viewer.

## Quick start

```bash
# Install from source
cargo install --git https://github.com/jackycamp/enza.git --root ~/.local

# Show help
enza -h

# Navigate to a repo on your machine
cd ~/some/project

# Show the diff
enza diff

# Alternatively, launch enza and choose a suggestion.
enza
```

## Install

### Install with script

TODO

### Install from releases

TODO

### Install from source

Install the latest source version with Cargo:

```sh
cargo install --git https://github.com/jackycamp/enza.git --root ~/.local
```

Or install from a local checkout:

```sh
git clone https://github.com/jackycamp/enza.git
cd enza
cargo install --path . --root ~/.local
```

Cargo installs the `enza` binary into Cargo's bin directory, usually `~/.cargo/bin`.
If `enza` is not found after installing, add that directory to your shell `PATH`.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
```

Check the install:

```sh
enza --version
```

## Features and Usage

TODO

## Build and run on macOS

Install the system build tools:

```sh
xcode-select --install
```

Install Rust with `rustup` if it is not already installed:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Build and run:

```sh
cargo build
cargo run
```

Build an optimized binary:

```sh
cargo build --release
./target/release/enza
```

Install it into Cargo's bin directory:

```sh
cargo install --path .
enza
```

If the build fails while compiling native dependencies, install Homebrew packages for `pkg-config` and OpenSSL:

```sh
brew install pkg-config openssl
```

## Build and run on Linux

Install common build dependencies.

Debian or Ubuntu:

```sh
sudo apt update
sudo apt install build-essential pkg-config libssl-dev git curl
```

Build and run:

```sh
cargo build
cargo run
```

Build an optimized binary:

```sh
cargo build --release
./target/release/enza
```

Install it into Cargo's bin directory:

```sh
cargo install --path .
enza
```

## Development checks

```sh
cargo fmt --check
cargo test
```
