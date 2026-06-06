# Contributing to Takit

Thanks for your interest in improving Takit! This is a small, focused app — the goal is
to stay lightweight and simple. Contributions that keep it that way are very welcome.

## Getting set up

1. Install [Rust](https://rustup.rs).
2. Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your
   OS. On Linux that means the WebKitGTK / AppIndicator dev packages — the exact list is
   in [`.github/workflows/ci.yml`](.github/workflows/ci.yml).
3. Install the Tauri CLI: `cargo install tauri-cli --version "^2"`.
4. Run the app: `cargo tauri dev`.

No Node.js or JavaScript tooling is needed — the frontend in [`src/`](src/) is plain
HTML/CSS/JS that Tauri serves directly.

## Before you open a PR

Run the same checks CI runs, from the `src-tauri/` directory:

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo build
```

## Guidelines

- **Keep it light.** Avoid adding heavy dependencies or a frontend framework.
- **Match the surrounding style.** Small, readable functions; comment the *why*.
- **One change per PR** where possible — it makes review easier.
- **Don't commit** `src-tauri/target/` or platform build artifacts (see `.gitignore`).

## Reporting bugs

Open an issue with your OS, the app version, the link/site (if shareable), and the error
text shown in the download row. For download failures, updating yt-dlp (Settings →
*Update yt-dlp*) fixes a large share of them.

## Code of conduct

Be kind and constructive. We're all here to make a nice little tool.
