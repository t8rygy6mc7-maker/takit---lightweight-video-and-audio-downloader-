<div align="center">

<img src="app-icon.png" width="120" alt="Takit icon" />

# Takit

**A lightweight, cross-platform video & audio downloader.**

Paste a link, pick *video* or *audio*, and download. Takit stays out of the way —
it runs quietly in the system tray and uses very little memory.

[![CI](https://github.com/t8rygy6mc7-maker/takit/actions/workflows/ci.yml/badge.svg)](https://github.com/t8rygy6mc7-maker/takit/actions/workflows/ci.yml)
[![Release](https://github.com/t8rygy6mc7-maker/takit/actions/workflows/release.yml/badge.svg)](https://github.com/t8rygy6mc7-maker/takit/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

</div>

---

## Features

- 🎬 **Video** or 🎵 **audio** — one click to switch. Pick a quality (up to 4K) or an
  audio format (MP3, M4A, Opus, FLAC, WAV).
- 📋 **Paste & go** — paste a link, press Enter. Optional preview shows the title and
  thumbnail before you commit.
- 🪶 **Lightweight** — built with [Tauri](https://tauri.app): a tiny native binary that
  uses your OS's built-in webview instead of bundling a browser. Installers are a few MB.
- 🔕 **Background-friendly** — minimizes to the system tray and idles at near-zero CPU.
- ⚡ **Queue** — download several links at once with a configurable concurrency limit.
- 🖥️ **macOS, Windows, and Linux** — one codebase, native builds for each.
- 🔄 **Always current** — powered by [yt-dlp](https://github.com/yt-dlp/yt-dlp), which
  Takit can update with one click so new sites and fixes keep working.

## How it works

Takit is a thin, friendly front-end around two trusted tools:

- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)** — does the actual downloading.
- **[ffmpeg](https://ffmpeg.org)** — extracts audio and merges high-quality streams.

To keep the app tiny, these aren't bundled into the installer. On first launch Takit
offers to download them (≈ 40–110 MB, one time) into its own app-data folder — so they
never touch your system. If you already have `yt-dlp` and/or `ffmpeg` on your `PATH`,
Takit will use those instead.

## Install

Grab the installer for your platform from the [**Releases**](https://github.com/t8rygy6mc7-maker/takit/releases) page:

| Platform | File |
| --- | --- |
| **macOS** | `.dmg` (universal — Apple Silicon & Intel) |
| **Windows** | `.exe` (NSIS installer) or `.msi` |
| **Linux** | `.AppImage`, `.deb`, or `.rpm` |

> **macOS / Windows note:** the app isn't code-signed yet, so the OS may warn the first
> time. On macOS, right-click → *Open*; on Windows, *More info → Run anyway*.

## Build from source

You need [Rust](https://rustup.rs) and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
(on Linux, the WebKitGTK/AppIndicator dev packages — see [`ci.yml`](.github/workflows/ci.yml)).
No Node.js or bundler is required — the UI is plain HTML/CSS/JS.

```bash
# Install the Tauri CLI once
cargo install tauri-cli --version "^2"

# Run in development
cargo tauri dev

# Produce an installer for the current platform
cargo tauri build
```

Regenerate the icon set (optional) with `python3 tools/make_icons.py` or
`cargo tauri icon app-icon.png`.

## Releasing

Push a version tag and the [release workflow](.github/workflows/release.yml) builds and
attaches installers for all three platforms to a draft GitHub Release:

```bash
git tag v0.1.0
git push origin v0.1.0
```

## Project layout

```
src/                 # Frontend (vanilla HTML/CSS/JS, no build step)
src-tauri/
  src/
    lib.rs           # App wiring: state, commands, tray, window lifecycle
    bins.rs          # Locate/download/update yt-dlp & ffmpeg
    download.rs      # Download queue, yt-dlp invocation, progress parsing
    settings.rs      # Persisted user settings
  tauri.conf.json    # Tauri configuration
tools/make_icons.py  # Icon generator (stdlib only)
.github/workflows/   # CI + release automation
```

## Legal

Takit is a tool. **You are responsible for how you use it.** Only download content you
have the right to, and respect the terms of service of the sites you use and the rights
of content creators. This project is not affiliated with yt-dlp, ffmpeg, or any video
platform.

## License

[MIT](LICENSE) © Takit contributors.
