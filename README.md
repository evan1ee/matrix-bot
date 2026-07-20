# Matrix Hello Bot (Rust) 🦀

A modern, fast, and feature-packed "Hello World" Matrix bot written in Rust using the official [`matrix-sdk`](https://github.com/matrix-org/matrix-rust-sdk).

## Features

- 🤖 **Interactive Bot Commands**: Responds to `!hello`, `!ping`, and `!help`.
- 📩 **Auto-Join Invites**: Automatically accepts room invitations when users invite the bot.
- 💬 **Formatted HTML & Plain Text Messages**: Sends styled Matrix notice responses.
- ⚙️ **Environment Configuration**: Easy configuration via `.env` file or environment variables.
- 📝 **Structured Logging**: Built-in `tracing` logging with customizable filters.

---

## Prerequisites

- [Rust & Cargo](https://rustup.rs/) (edition 2021+)
- A Matrix Account on any Matrix Homeserver (e.g. `matrix.org`, local Synapse, or Conduit).

---

## Setup & Running

### 1. Clone & Navigate to Project

```bash
cd d:/01_Data/code/matrix-hello-bot
```

### 2. Configure Environment

Copy `.env.example` to `.env`:

```bash
cp .env.example .env
```

Edit `.env` with your Matrix account details:

```env
MATRIX_HOMESERVER=https://matrix.org
MATRIX_USERNAME=your_bot_username
MATRIX_PASSWORD=your_bot_password
RUST_LOG=info,matrix_hello_bot=debug,matrix_sdk=info
```

### 3. Build & Run

```bash
cargo run
```

---

## Usage & Bot Commands

Once the bot is running:

1. **Invite the bot** to any room (or start a Direct Message with it). The bot will automatically accept the invitation.
2. Type any of the following commands in the room:

| Command | Action | Example Response |
|---------|--------|------------------|
| `!hello` or `hello` | Friendly greeting | 👋 Hello, alice! Greetings from **matrix-hello-bot** written in Rust! 🦀 |
| `!hello <name>` | Custom greeting | 👋 Hello, **World**! Greetings from Rust! 🦀 |
| `!ping` | Check bot status | 🏓 **Pong!** Matrix Hello Bot is active and running! |
| `!help` | List available commands | Displays command menu |

---

## Code Overview

- [`Cargo.toml`](file:///d:/01_Data/code/matrix-hello-bot/Cargo.toml): Project metadata and dependencies (`matrix-sdk`, `tokio`, `tracing`, `dotenvy`).
- [`src/main.rs`](file:///d:/01_Data/code/matrix-hello-bot/src/main.rs): Core Matrix client initialization, authentication, event listeners (`OriginalSyncRoomMessageEvent`, `StrippedRoomMemberEvent`), and sync execution loop.
- [`.env.example`](file:///d:/01_Data/code/matrix-hello-bot/.env.example): Environment variable template.
