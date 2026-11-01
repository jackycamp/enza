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

These commands install `enza` into `~/.local/bin`.
Add that directory to your shell `PATH`:

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

Press Ctrl+C on a file header or anywhere in its body to collapse the file.
Press Ctrl+C on the collapsed header to reopen it. A green check mark identifies
files you have viewed by collapsing them. The check mark remains after reopening
until you exit the session.

Unread agent replies replace the file's `A`, `M` or `D` status marker with a
purple `●`. Reading the reply restores the status marker. Containing folders
also show a purple `●` until their replies are read. Press `ga` for the next
unread reply or `gA` for the previous unread reply.
These commands wrap in diff order and work from either pane outside the note
composer. A jump opens the file and thread and focuses the newest reply.
Focusing the newest reply in an expanded thread also marks it read. Selecting
the file or seeing a reply elsewhere in the pane does not mark it read.
Unread markers last for the current session.

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
