# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a high-performance Rust-based RTSP streaming server that connects to multiple IP cameras and streams video to web browsers via WebSockets. It's designed for integration with Siemens WinCC Unified but works as a standalone streaming solution.

## Development Commands

### Building and Running
```bash
# Development with debug logging
RUST_LOG=debug cargo run

# Production build
cargo build --release
./target/release/rtsp-streaming-server

# Run with custom config
cargo run -- --config config.toml

# Quick test with timeout (for testing)
timeout 5 cargo run -- -c config-test.toml
```

### Code Quality
```bash
# Check for compilation errors
cargo check

# Build the project
cargo build

# Format code (if rustfmt is configured)
cargo fmt

# Lint code (if clippy is configured)
cargo clippy
```

## Architecture Overview

### Core Components
- **Main Server (`main.rs`)**: Axum web server with WebSocket support, camera management, and graceful shutdown
- **Configuration (`config.rs`)**: Hot-reloadable config system with main `config.toml` + individual camera JSON files in `cameras/`
- **Video Streaming (`video_stream.rs`, `rtsp_client.rs`)**: RTSP connections using `retina` crate + FFmpeg transcoding via subprocess
- **WebSocket Handler (`websocket.rs`)**: Real-time binary frame streaming to browser clients with token authentication
- **Recording System (`recording.rs`, `database.rs`)**: Per-camera SQLite databases for frame storage with session management
- **MQTT Integration (`mqtt.rs`)**: Real-time camera status and image publishing
- **Control System (`control.rs`)**: WebSocket/REST APIs for recording control and playback

### Data Flow
```
RTSP Camera → FFmpeg Process → Frame Transcoding → Broadcast Channel → WebSocket Clients
                                      ↓
                              Recording System (Optional)
                                      ↓
                                SQLite Database
```

### Configuration Architecture
- **Main config**: `config.toml` for server settings, global FFmpeg params, MQTT, recording settings
- **Camera configs**: `cameras/*.json` files with per-camera settings, hot-reloaded via file watcher
- **Dynamic loading**: Changes to camera files automatically restart affected streams

### URL Structure
- `/dashboard` - Multi-camera overview
- `/admin` - Camera management interface  
- Per-camera endpoints (for camera with `path = "/cam1"`):
  - `/cam1` - Camera test page
  - `/cam1/stream` - Video streaming interface
  - `/cam1/control` - Recording control interface
  - `/cam1/live` - WebSocket-only streaming
- API endpoints: `/api/status`, `/api/cameras`, `/api/admin/cameras/*`

## Key Technical Details

### Async Architecture
- **Tokio runtime**: 16 worker threads, full async throughout
- **Broadcast channels**: Efficient frame distribution to multiple WebSocket clients
- **Connection management**: Graceful handling of RTSP reconnections and client disconnects

### Frame Processing
- **FFmpeg integration**: External subprocess with configurable parameters
- **Buffer management**: Configurable frame buffer sizes per camera
- **Performance monitoring**: FPS tracking and duplicate frame detection

### Database Design
- **Per-camera SQLite**: Individual databases in `recordings/` directory
- **Session-based**: Recording sessions with start/stop timestamps
- **Automatic cleanup**: Configurable age-based deletion of old recordings

### Security
- **Token authentication**: Per-camera tokens for WebSocket access (query params or headers)
- **Admin tokens**: For camera management operations
- **TLS support**: Optional HTTPS with custom certificates

## Configuration Files

### Adding a New Camera
1. Create `cameras/new_camera.json` with camera configuration
2. Server automatically detects and loads the new camera
3. Camera becomes available at the configured path

### Camera Configuration Example
```json
{
  "name": "Camera 1",
  "path": "/cam1",
  "rtsp_url": "rtsp://camera-ip:554/stream",
  "token": "secure-token",
  "transcoding": {
    "enabled": true,
    "format": "mjpeg",
    "fps": 15,
    "resolution": "640x480"
  }
}
```

## Important Dependencies
- **`retina`**: RTSP client library for camera connections
- **`axum`**: Web framework with WebSocket support
- **`sqlx`**: SQLite database operations
- **`rumqttc`**: MQTT client for status publishing
- **`notify`**: File system watching for config hot-reload
- **External FFmpeg**: Must be installed system-wide for video transcoding

## Development Notes

### Testing
- No formal test suite currently exists
- Manual testing via web interfaces at `/dashboard` and individual camera pages
- Use `timeout` commands for quick testing without long-running processes

### Adding New Features
- **Camera features**: Modify camera JSON schema and `config.rs` parsing
- **Streaming features**: Extend `video_stream.rs` and `websocket.rs`
- **Recording features**: Modify `recording.rs` and database schema
- **Web interface**: Static files served from `static/` directory

### Common Issues
- **FFmpeg path**: Ensure FFmpeg is in system PATH
- **RTSP connections**: Check camera credentials and network connectivity
- **WebSocket connections**: Verify token authentication for camera access
- **Port conflicts**: Default server runs on port 8080, configurable in `config.toml`

### File Watching
- Server watches `cameras/` directory for changes
- Configuration changes trigger automatic stream restarts
- Admin interface provides web-based camera management