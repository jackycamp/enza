# enza

A fast and efficient terminal-based code diff viewer.

> enza is still a work-in-progress and is not yet stable, features and functionality are subject to change without notice. Thank you.

<img width="1902" height="1035" alt="image" src="https://github.com/user-attachments/assets/57919f33-2b19-4de6-86af-c4d80063d447" />

<img width="1906" height="1037" alt="image" src="https://github.com/user-attachments/assets/3e5e5de9-2ba5-4dbe-8888-f08fe45bc8a4" />

## Quick start

```bash
# Install from source
cargo install --git https://github.com/jackycamp/enza.git --root ~/.local

# Add the install directory to your PATH
export PATH="$HOME/.local/bin:$PATH"

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

These commands install `enza` into `~/.local/bin`. Ensure that directory is added to your shell `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Add the same `export` line to your shell configuration, such as `~/.zshrc`, to keep it for future sessions.

To reinstall after changing your local checkout, run this from the checkout:

```sh
cargo install --path . --root ~/.local
```

Check the install:

```sh
enza --version
```

## Features and Usage

### Notes

You can leave a note anywhere in the diff by typing `n`, entering some text, and pressing enter. Press escape to exit note mode. By default, the note is associated with the line your cursor was at when you pressed `n`.

You can also select sections of the diff by pressing `v` and then `n` to leave a note associated with your selection.

### Message an agent

To message an agent you can press `n` to enter note mode and then you can message your agent of choice by typing `@codex` or `@claude` with your question or request.

When the agent has responded, you will see a purple `●` next to the file's name in the file explorer and in the file header of the diff. From here, you can press `ga` to go to the next unread agent reply or `gA` for the previous unread agent reply.

### Collapse files

Press ctrl+c on a file header or anywhere in its body to collapse the file, marking it as "viewed." You can expand the file by pressing ctrl+c on the collapsed file header. A green check mark will appear next to the file header indicating that you've marked it as read.

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

Install it into `~/.local/bin`:

```sh
cargo install --path . --root ~/.local
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

Install it into `~/.local/bin`:

```sh
cargo install --path . --root ~/.local
enza
```

## Development checks

```sh
cargo fmt --check
cargo test
```
