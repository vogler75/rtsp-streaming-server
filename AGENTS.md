# Repository Guidelines

## Project Structure & Modules
- `src/`: Rust sources (e.g., `rtsp_client.rs`, `websocket_*`, `api_*`, `recording.rs`, `mp4.rs`, `database.rs`, `ptz.rs`).
- `static/`: Browser UI assets (dashboard, control, stream pages: `dashboard.html/js`, `control.html`).
- `homepage/`: Marketing/landing page (not required for runtime).
- `cameras/`: Per‑camera JSON configs (watched live). Example: `cameras/cam1.json`.
- `recordings/`: Recording database/files (auto-created; ignored by git).
- `certs/`: TLS certificates when HTTPS is enabled.
- `config.json`: Main server config; see examples in repo.

## Build, Run, and Dev
- Build: `cargo build` (use `--release` for production).
- Run: `cargo run -- --config config.json` (add `--verbose` for detailed logs).
- Lint: `cargo clippy -- -D warnings` (treat lints as errors).
- Format: `cargo fmt` (run before committing).
- Sanity check: `cargo check`.
- Prerequisite: FFmpeg must be on `PATH` (`ffmpeg -version`).

## Coding Style & Naming
- Rust 2021 edition; 4‑space indentation.
- Modules/files: `snake_case` (e.g., `video_stream.rs`).
- Types/traits/enums: `UpperCamelCase`; functions/vars: `snake_case`.
- Prefer `anyhow`/`thiserror` for errors and `tracing` for logs.
- Keep modules focused; colocate small tests with modules.

## Testing Guidelines
- Framework: Rust built‑in tests (`#[cfg(test)]`).
- Add unit tests near code; integration tests under `tests/` if needed.
- Run tests: `cargo test`.
- Manual checks: verify key routes (`/dashboard`, `/<cam>/stream`, `/<cam>/control`) and database/recording flows.

## Commit & Pull Requests
- Commits: concise, imperative subject (e.g., "Add throughput tracker"), include context in body when needed.
- Link issues in PRs; describe changes, rationale, and risk.
- Include test plan (commands, routes exercised) and screenshots for UI changes (`static/`/`homepage/`).
- Note any config/schema changes (e.g., new `config.json` fields) and provide migration notes.

## Security & Config Tips
- Do not commit real credentials, tokens, or certs. Use placeholders in examples.
- Camera URLs often include passwords; prefer environment‑specific `config.json` and files in `cameras/` (git‑ignored patterns already cover sensitive outputs).
- Enable HTTPS by placing PEM files in `certs/` and setting `server.tls` in `config.json`.
- Admin access is token‑protected; keep tokens long and random.
