# Claude Code Prompt: Low-Latency RTSP Camera Streaming Server in Rust

## Project Overview
Build a high-performance, low-latency video streaming server that:
1. Connects to RTSP cameras
2. Transcodes video streams for browser compatibility
3. Serves video through WebSockets with minimal latency
4. Provides a simple HTML interface for viewing streams

## Technical Requirements

### Core Technologies
- **Language**: Rust (confirmed as excellent choice for performance and safety)
- **RTSP Client**: Use the `retina` crate (production-ready RTSP library in Rust)
- **Video Processing**: FFmpeg for transcoding when needed
- **WebSocket Server**: `tokio-tungstenite` or `tungstenite` for WebSocket implementation
- **Web Framework**: `actix-web` or `axum` for HTTP server
- **Async Runtime**: `tokio` for async operations

### Architecture Design

```
RTSP Camera → Rust Server → WebSocket → Browser
                ↓
           [Transcoding]
                ↓
         [Format Options]
         • MJPEG (lowest latency for simple streaming)
         • H.264 chunks (better compression)
         • Raw frames (maximum control)
```

## Implementation Approach

### Option 1: MJPEG over WebSockets (Simplest, Low Latency ~100-200ms)
**Pros**: Simple implementation, wide browser support, no special codecs needed
**Cons**: Higher bandwidth usage

### Option 3: WebRTC Integration (Lowest Latency ~50-150ms)
**Pros**: Absolute lowest latency, native browser support
**Cons**: Most complex implementation, requires STUN/TURN setup

## Recommended Implementation Plan

### Phase 1: Basic MJPEG Streaming
1. Create Rust server with `actix-web` or `axum`
2. Use `retina` crate to connect to RTSP camera
3. Convert frames to JPEG using `image` crate or FFmpeg
4. Stream JPEG frames over WebSocket
5. Create HTML client with Canvas rendering

### Phase 2: Add FFmpeg Integration
1. Integrate FFmpeg for advanced transcoding options
2. Support multiple codec outputs
3. Add dynamic quality adjustment
4. Implement frame-by-frame mode for ultra-low latency

### Phase 3: WebRTC Support (Optional)
1. Add WebRTC support using `webrtc-rs` crate
2. Implement WHIP (WebRTC-HTTP Ingestion Protocol)
3. Create signaling server
4. Update HTML client for WebRTC

## Project Structure

```
rtsp-streaming-server/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point and server setup
│   ├── rtsp_client.rs       # RTSP connection handling
│   ├── transcoder.rs        # Video transcoding logic
│   ├── websocket.rs         # WebSocket server
│   ├── ffmpeg.rs           # FFmpeg integration
│   └── config.rs           # Configuration management
├── static/
│   ├── index.html          # Viewer interface
│   ├── player.js           # WebSocket client and video player
│   └── style.css           # Basic styling
└── README.md
```

## Key Dependencies (Cargo.toml)

```toml
[dependencies]
# Core async runtime
tokio = { version = "1", features = ["full"] }

# Web server
actix-web = "4"
actix-ws = "0.2"

# RTSP handling
retina = "0.4"

# WebSocket
tungstenite = "0.21"
tokio-tungstenite = "0.21"

# Image processing
image = "0.24"
bytes = "1"

# Configuration
serde = { version = "1", features = ["derive"] }
serde_json = "1"
config = "0.13"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Optional: WebRTC support
# webrtc = "0.9"

# Error handling
anyhow = "1"
thiserror = "1"
```

## Core Implementation Components

### 1. RTSP Client Module
- Connect to RTSP camera using `retina` crate
- Handle authentication (if required)
- Support both TCP and UDP transport
- Implement reconnection logic
- Buffer management for smooth playback

### 2. Transcoding Module
- Decode H.264/H.265 from RTSP
- Options for output format:
  - MJPEG: Encode each frame as JPEG
  - H.264 chunks: Pass through or re-encode
  - Raw RGB: For maximum flexibility
- Dynamic quality adjustment based on network conditions
- Frame dropping for latency control

### 3. WebSocket Server
- Accept multiple client connections
- Efficient broadcasting to multiple viewers
- Backpressure handling
- Client state management
- Automatic reconnection support

### 4. FFmpeg Integration (Optional but Recommended)
- Use `std::process::Command` to spawn FFmpeg
- Pipe RTSP input through FFmpeg for transcoding
- Output to stdout and capture in Rust
- Support various codec conversions
- Example FFmpeg command:
```bash
ffmpeg -rtsp_transport tcp -i rtsp://camera_ip/stream \
       -c:v mjpeg -q:v 5 -f mjpeg pipe:1
```

### 5. HTML/JavaScript Client
- WebSocket connection management
- Render video on Canvas or Video element
- Handle different stream formats:
  - MJPEG: Draw each JPEG on canvas
  - H.264: Use Media Source Extensions or decoder library
  - Raw: Direct canvas manipulation
- Latency measurement and display
- Reconnection logic
- Multiple camera support

## Configuration File (config.toml)

```toml
[server]
host = "0.0.0.0"
port = 8080
websocket_port = 8081

[rtsp]
url = "rtsp://admin:password@192.168.1.100:554/stream"
transport = "tcp"  # or "udp"
reconnect_interval = 5  # seconds

[transcoding]
output_format = "mjpeg"  # or "h264", "raw"
quality = 85  # for MJPEG
framerate = 30
resolution = "1920x1080"
max_latency_ms = 200

[ffmpeg]
enabled = true
path = "/usr/bin/ffmpeg"
extra_args = ["-tune", "zerolatency", "-preset", "ultrafast"]

[performance]
worker_threads = 4
max_clients = 100
frame_buffer_size = 10
```

## Performance Optimization Tips

1. **Use Non-blocking I/O**: Leverage Tokio for all I/O operations
2. **Zero-copy where possible**: Use `bytes::Bytes` for frame data
3. **Frame buffering**: Implement ring buffer for smooth playback
4. **Parallel processing**: Use Rayon for parallel frame encoding
5. **Memory pool**: Reuse buffers to reduce allocations
6. **TCP vs UDP**: Test both for RTSP transport (TCP more reliable, UDP lower latency)
7. **Tune FFmpeg**: Use `-tune zerolatency -preset ultrafast` flags
8. **WebSocket compression**: Disable for lowest latency
9. **Browser optimizations**: Use OffscreenCanvas and Web Workers

## Testing Strategy

1. **Unit tests**: Test individual modules
2. **Integration tests**: Test RTSP → WebSocket pipeline
3. **Load testing**: Simulate multiple concurrent viewers
4. **Latency measurement**: End-to-end latency testing
5. **Network conditions**: Test under various network conditions
6. **Browser compatibility**: Test on Chrome, Firefox, Safari, Edge

## Deployment Considerations

1. **Docker support**: Create Dockerfile for easy deployment
2. **SSL/TLS**: Add HTTPS and WSS support for production
3. **Authentication**: Implement token-based auth for WebSocket
4. **Monitoring**: Add Prometheus metrics
5. **Logging**: Structured logging with tracing
6. **Health checks**: HTTP endpoints for monitoring
7. **Resource limits**: CPU and memory constraints
8. **Horizontal scaling**: Support multiple server instances

## Example Implementation Snippet

```rust
// Basic WebSocket frame sender
async fn send_frame_to_clients(
    frame: Bytes,
    clients: &Arc<Mutex<Vec<WebSocketClient>>>
) -> Result<()> {
    let clients = clients.lock().await;
    let mut disconnected = vec![];
    
    for (idx, client) in clients.iter().enumerate() {
        if let Err(_) = client.send(Message::Binary(frame.clone())).await {
            disconnected.push(idx);
        }
    }
    
    // Remove disconnected clients
    for idx in disconnected.iter().rev() {
        clients.remove(*idx);
    }
    
    Ok(())
}
```

## Alternative Approaches to Consider

4. **WASM decoder**: Compile video decoder to WASM for browser
5. **Multiple quality streams**: Adaptive bitrate streaming

## Success Metrics

- **Latency**: < 200ms for MJPEG, < 500ms for H.264
- **CPU usage**: < 30% per stream on modern hardware
- **Memory usage**: < 100MB per connected client
- **Concurrent viewers**: Support 50+ simultaneous connections
- **Reliability**: 99.9% uptime with automatic reconnection

## Additional Features to Implement

1. **Recording**: Save streams to disk
2. **Motion detection**: Basic computer vision
3. **PTZ control**: Camera pan/tilt/zoom control
4. **Multiple cameras**: Dashboard view
5. **Authentication**: User management
6. **Analytics**: Viewer statistics
7. **Thumbnails**: Periodic snapshots
8. **Audio support**: If camera has audio
9. **Mobile app**: React Native or Flutter client
10. **Cloud storage**: Upload recordings to S3/GCS

## Resources and References

- Retina RTSP library: https://github.com/scottlamb/retina
- Actix Web: https://actix.rs/
- Tokio: https://tokio.rs/
- WebRTC-rs: https://github.com/webrtc-rs/webrtc
- FFmpeg RTSP guide: https://ffmpeg.org/ffmpeg-protocols.html#rtsp
- WebSocket MDN: https://developer.mozilla.org/en-US/docs/Web/API/WebSocket
- Media Source Extensions: https://developer.mozilla.org/en-US/docs/Web/API/Media_Source_Extensions_API

This comprehensive prompt should give Claude Code all the necessary context and technical details to build a robust, low-latency RTSP camera streaming server in Rust.