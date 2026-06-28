# Ping Pong Latency

A small Tauri desktop app for tracking ping latency against IP addresses and domain names.

## Current shape

- Windows-first Tauri app with a React/Vite frontend.
- SQLite persistence in the app data directory.
- Add IP addresses or hostnames.
- Ping intervals: 1, 2, 5, or 10 seconds.
- Graph windows: 1 minute, 5 minutes, 15 minutes, 1 hour, 6 hours, and 12 hours.
- Tracks current latency, average latency, max latency, and packet loss.
- Logs high latency and failed pings without showing desktop notifications.

## Development setup

Install the normal Tauri prerequisites:

- Node.js
- Rust
- Microsoft Visual Studio Build Tools with the Desktop development with C++ workload
- WebView2 Runtime

Then install dependencies and run the app:

```powershell
pnpm install
pnpm tauri:dev
```

## Build

```powershell
pnpm tauri:build
```

## macOS later

The app is structured so the UI, SQLite model, and command API are platform-neutral. The backend currently calls the system `ping` command and has Windows and Unix-style argument branches. For Apple Silicon universal binaries later, build from macOS with both targets installed:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm tauri:build -- --target universal-apple-darwin
```
