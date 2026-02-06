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
cargo run -- --config config.json

# Quick test with timeout (for testing)
timeout 5 cargo run -- -c config-test.json
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
- **Configuration (`config.rs`)**: Hot-reloadable config system with main `config.json` + individual camera JSON files in `cameras/`
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
- **Main config**: `config.json` for server settings, global FFmpeg params, MQTT, recording settings
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
- **Configurable backends**: SQLite (per-camera files) or PostgreSQL (shared or per-camera databases)
- **Session-based**: Recording sessions with start/stop timestamps  
- **Automatic cleanup**: Configurable age-based deletion of old recordings
- **Database abstraction**: Single interface supports both SQLite and PostgreSQL seamlessly

### Security
- **Token authentication**: Per-camera tokens for WebSocket access (query params or headers)
- **Admin tokens**: For camera management operations
- **TLS support**: Optional HTTPS with custom certificates

## Configuration Files

### Database Configuration

The server supports both SQLite and PostgreSQL databases for recording storage:

#### SQLite Configuration (Default)
```json
{
  "recording": {
    "database_type": "sqlite",
    "database_path": "recordings"
  }
}
```
- Creates per-camera SQLite files: `recordings/{camera_id}.db`
- No additional setup required

#### PostgreSQL Configuration - Per-Camera Databases
```json
{
  "recording": {
    "database_type": "postgresql", 
    "database_url": "postgres://user:password@localhost/"
  }
}
```
- Creates separate databases: `rtsp_cam1`, `rtsp_cam2`, etc.
- Isolates camera data for better organization

#### PostgreSQL Configuration - Shared Database
```json
{
  "recording": {
    "database_type": "postgresql",
    "database_url": "postgres://user:password@localhost/surveillance"
  }
}
```
- All cameras share single database with `camera_id` discrimination
- More efficient for large deployments

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
- **`sqlx`**: Database operations (SQLite and PostgreSQL support)
- **`rumqttc`**: MQTT client for status publishing
- **`notify`**: File system watching for config hot-reload
- **External FFmpeg**: Must be installed system-wide for video transcoding
- **PostgreSQL** (optional): For PostgreSQL database backend

## Development Notes

### Testing
- No formal test suite currently exists
- Manual testing via web interfaces at `/dashboard` and individual camera pages
- Use `timeout` commands for quick testing without long-running processes

### Adding New Features

**IMPORTANT: Full-stack checklist.** Every feature that touches backend code likely also needs UI changes. Always work through this checklist before considering a feature complete:

#### When adding or changing a config field (`src/config.rs`):
1. **Rust struct**: Add field to the relevant struct (`RecordingConfig`, `CameraConfig`, `ServerConfig`, etc.) with `#[serde(default)]` for backward compatibility
2. **Default value**: Update `Config::default()` to include the new field
3. **Dashboard HTML** (`static/dashboard.html`): Add a form input/select in the matching section of the Server Configuration modal (Recording Settings, Server Settings, MQTT, etc.)
4. **Dashboard JS — populate** (`static/dashboard.js` → `populateServerConfigForm()`): Read the field from the config response and set it on the new form element
5. **Dashboard JS — collect** (`static/dashboard.js` → `collectServerConfigFromForm()`): Include the field in the object sent back to the API on save
6. **Per-camera override** (if applicable): Also add to `CameraRecordingConfig` struct, the camera edit form in `dashboard.html` (`populateForm()` in JS), and camera save logic

#### When adding a new API endpoint:
1. **Rust handler**: Implement in the appropriate `api_*.rs` or `handlers.rs` file
2. **Router**: Register the route in `main.rs`
3. **Dashboard UI**: Add buttons/links/forms in `static/dashboard.html` + JS calls in `static/dashboard.js`
4. **Camera pages** (if per-camera): Update `static/control.html` or `static/stream.html` as needed

#### General feature areas:
- **Camera features**: Modify camera JSON schema and `config.rs` parsing
- **Streaming features**: Extend `video_stream.rs` and `websocket.rs`
- **Recording features**: Modify `recording.rs` and database schema
- **Web interface**: Static files served from `static/` directory

### Common Issues
- **FFmpeg path**: Ensure FFmpeg is in system PATH
- **RTSP connections**: Check camera credentials and network connectivity
- **WebSocket connections**: Verify token authentication for camera access
- **Port conflicts**: Default server runs on port 8080, configurable in `config.json`
- **PostgreSQL connections**: Ensure PostgreSQL server is running and credentials are correct
- **Database permissions**: PostgreSQL user needs CREATE DATABASE permissions for per-camera databases

### File Watching
- Server watches `cameras/` directory for changes
- Configuration changes trigger automatic stream restarts
- Admin interface provides web-based camera management