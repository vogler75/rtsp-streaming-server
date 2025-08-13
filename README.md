# RTSP Video Streaming Server

A high-performance, low-latency video streaming server built in Rust that connects to RTSP cameras and streams video to web browsers via WebSockets.

## Features

- **Multi-Camera Support**: Stream from multiple RTSP cameras simultaneously
- **RTSP Camera Support**: Connect to RTSP cameras and streams
- **WebSocket Streaming**: Low-latency video streaming via WebSockets  
- **MJPEG Output**: Converts video to JPEG frames for browser compatibility
- **HTTPS/TLS Support**: Secure connections with configurable certificates
- **Web Interface**: Simple HTML client for viewing streams
- **Real-time Stats**: FPS, frame count, and latency monitoring
- **Auto-reconnection**: Automatic reconnection for both RTSP and WebSocket connections
- **Per-Camera Debug**: Camera-specific logging with ID prefixes

## Quick Start

1. **Build and run the server**:
   ```bash
   cargo run
   ```

2. **Open your web browser** and navigate to:
   ```
   # For specific cameras
   http://localhost:8080/cam1
   http://localhost:8080/cam2
   http://localhost:8080/cam3
   ```
   
   For HTTPS (if TLS is enabled):
   ```
   https://localhost:8080/cam1
   ```

3. **Configure cameras** by editing `config.toml` (see Configuration section)

## Configuration

The server can be configured via the `config.toml` file:

### Server Configuration

```toml
[server]
host = "0.0.0.0"
port = 8080
cors_allow_origin = "null"  # or "*" for permissive, or specific origin

[server.tls]
enabled = false
cert_path = "certs/server.crt"
key_path = "certs/server.key"
```

### Global Transcoding Settings

```toml
[transcoding]
output_format = "mjpeg"
quality = 50
capture_framerate = 0      # 0 = max available from camera
send_framerate = 10        # FPS sent to clients
channel_buffer_size = 1    # Frame buffer size (1 = only latest)
allow_duplicate_frames = false  # Send each frame only once
debug_capture = true       # Show capture rate debug info
debug_sending = false      # Show sending rate debug info
```

### Multi-Camera Configuration

Configure multiple cameras, each with its own path and settings:

```toml
# Camera 1
[cameras.cam1]
path = "/cam1"
url = "rtsp://username:password@192.168.1.182:554/stream1"
transport = "tcp"
reconnect_interval = 5
chunk_read_size = 32768
ffmpeg_buffer_size = 65536

# Camera 2
[cameras.cam2]
path = "/cam2"
url = "rtsp://username:password@192.168.1.171:554/stream1"
transport = "tcp"
reconnect_interval = 5
chunk_read_size = 32768
ffmpeg_buffer_size = 65536

# Camera 3 with custom transcoding settings
[cameras.cam3]
path = "/cam3"
url = "rtsp://username:password@192.168.1.188:554/stream2"
transport = "tcp"
reconnect_interval = 5
chunk_read_size = 32768
ffmpeg_buffer_size = 65536
# Optional: Override global transcoding settings for this camera
# quality = 30
# send_framerate = 5
```

### Configuration Options

#### Server Options
- **server.host**: Server bind address (default: "0.0.0.0")
- **server.port**: Server port (default: 8080)
- **server.cors_allow_origin**: CORS allowed origin (default: "*")
- **server.tls.enabled**: Enable HTTPS/TLS (default: false)
- **server.tls.cert_path**: Path to SSL certificate file
- **server.tls.key_path**: Path to SSL private key file

#### Camera Options
- **path**: URL path for this camera (e.g., "/cam1")
- **url**: RTSP camera URL with credentials
- **transport**: Transport protocol - "tcp" or "udp"
- **reconnect_interval**: Seconds between reconnection attempts
- **chunk_read_size**: Bytes to read at once from FFmpeg
- **ffmpeg_buffer_size**: FFmpeg RTSP buffer size in bytes

#### Transcoding Options
- **output_format**: Output format - currently "mjpeg"
- **quality**: JPEG quality (1-100)
- **capture_framerate**: Capture rate from camera (0 = max)
- **send_framerate**: Rate to send frames to clients
- **channel_buffer_size**: Number of frames to buffer
- **allow_duplicate_frames**: Send same frame multiple times
- **debug_capture**: Enable capture rate debug output
- **debug_sending**: Enable sending rate debug output

## WinCC Unified Integration

To integrate the video streaming server with Siemens WinCC Unified, you need to configure IIS to proxy the video streams through the same origin to avoid CORS issues.

### IIS Configuration for WinCC Unified

1. **Locate the web.config file** in:
   ```
   C:\Program Files\Siemens\Automation\WinCCUnified\SimaticUA
   ```

2. **Add the following rewrite rule** to the `<system.webServer><rewrite><rules>` section:

   ```xml
   <rule name="video">
       <match url="(.*)" />
       <conditions>
           <add input="{URL}" pattern="(.*)\/video\/(.*)" />
       </conditions>
       <action type="Rewrite" url="http://localhost:8080/{C:2}" />
   </rule>
   ```

   This rule will:
   - Match any URL containing `/video/`
   - Proxy the request to the video streaming server running on `localhost:8080`
   - Preserve the path after `/video/` (e.g., `/video/cam1` → `http://localhost:8080/cam1`)

3. **Access cameras through WinCC Unified** using:
   ```
   https://your-wincc-server/video/cam1
   https://your-wincc-server/video/cam2
   https://your-wincc-server/video/cam3
   ```

### Example: Complete IIS Rewrite Section

```xml
<system.webServer>
    <rewrite>
        <rules>
            <!-- Other existing rules -->
            
            <!-- Video streaming proxy rule -->
            <rule name="video">
                <match url="(.*)" />
                <conditions>
                    <add input="{URL}" pattern="(.*)\/video\/(.*)" />
                </conditions>
                <action type="Rewrite" url="http://localhost:8080/{C:2}" />
            </rule>
        </rules>
    </rewrite>
</system.webServer>
```

### Notes for WinCC Integration

- The video streaming server must be running on the same machine as WinCC Unified
- If running on a different machine, update the rewrite URL accordingly:
  ```xml
  <action type="Rewrite" url="http://video-server-ip:8080/{C:2}" />
  ```
- Ensure the IIS Application Request Routing (ARR) module is installed for proxy functionality
- The `/video/` prefix can be customized to match your requirements

## HTTPS/TLS Setup

For secure connections, you can enable HTTPS by generating SSL certificates and configuring the server.

### Generate Self-Signed Certificates (Development)

For development and testing, you can create self-signed certificates:

```bash
# Create certificates directory
mkdir -p certs

# Generate private key
openssl genrsa -out certs/server.key 2048

# Generate certificate signing request
openssl req -new -key certs/server.key -out certs/server.csr \
    -subj "/C=US/ST=State/L=City/O=Organization/CN=localhost"

# Generate self-signed certificate (valid for 365 days)
openssl x509 -req -days 365 -in certs/server.csr -signkey certs/server.key \
    -out certs/server.crt -extensions v3_req \
    -extfile <(echo -e "subjectAltName=DNS:localhost,IP:127.0.0.1")

# Clean up CSR file
rm certs/server.csr

# Set appropriate permissions
chmod 600 certs/server.key
chmod 644 certs/server.crt
```

### Generate Production Certificates

For production use, obtain certificates from a Certificate Authority (CA) like Let's Encrypt:

#### Using Certbot (Let's Encrypt)

```bash
# Install certbot (Ubuntu/Debian)
sudo apt install certbot

# Generate certificate for your domain
sudo certbot certonly --standalone -d yourdomain.com

# Copy certificates to project directory
sudo cp /etc/letsencrypt/live/yourdomain.com/fullchain.pem certs/server.crt
sudo cp /etc/letsencrypt/live/yourdomain.com/privkey.pem certs/server.key
sudo chown $USER:$USER certs/server.*
```

#### Using Custom CA Certificates

If you have certificates from another CA, copy them to the certs directory:

```bash
mkdir -p certs
cp your-certificate.pem certs/server.crt
cp your-private-key.pem certs/server.key
chmod 600 certs/server.key
chmod 644 certs/server.crt
```

### Enable HTTPS

Once you have certificates, enable HTTPS in your `config.toml`:

```toml
[server.tls]
enabled = true
cert_path = "certs/server.crt"
key_path = "certs/server.key"
```

The server will automatically:
- Load and validate the certificates on startup
- Start an HTTPS server instead of HTTP
- Support secure WebSocket connections (WSS)
- Display certificate information in the logs

### Browser Certificate Warnings

For self-signed certificates, browsers will show security warnings. To proceed:

1. **Chrome/Edge**: Click "Advanced" → "Proceed to localhost (unsafe)"
2. **Firefox**: Click "Advanced" → "Accept the Risk and Continue"
3. **Safari**: Click "Show Details" → "visit this website"

For production, use certificates from a trusted CA to avoid these warnings.

### Certificate Formats

The server supports PEM-encoded certificates and keys:
- **Certificate**: `.crt`, `.pem`, `.cert` files containing the public certificate
- **Private Key**: `.key`, `.pem` files containing the private key (unencrypted)

If you have PKCS#12 (.p12/.pfx) files, convert them:

```bash
# Extract certificate
openssl pkcs12 -in certificate.p12 -clcerts -nokeys -out server.crt

# Extract private key (will prompt for passwords)
openssl pkcs12 -in certificate.p12 -nocerts -nodes -out server.key
```

## Architecture

```
Multiple RTSP Cameras → Rust Server → WebSocket → Browser
         ↓                    ↓
    [Camera 1]          [Transcoding]
    [Camera 2]               ↓
    [Camera 3]          MJPEG Frames
         ↓                    ↓
   Individual Paths     Individual WebSocket
   (/cam1, /cam2)         Connections
```

### Components

- **Video Stream Manager** (`src/video_stream.rs`): Manages individual camera streams
- **RTSP Client** (`src/rtsp_client.rs`): Handles RTSP connections and frame reception
- **Transcoder** (`src/transcoder.rs`): Converts video frames to JPEG format
- **WebSocket Server** (`src/websocket.rs`): Manages WebSocket connections and frame broadcasting
- **Web Interface** (`static/index.html`): Browser-based video player

## Current Status

This is a working implementation with the following features:

✅ **Working**:
- Multi-camera support with individual paths
- Basic server architecture
- WebSocket streaming 
- MJPEG frame generation
- Web interface with real-time stats
- Configuration management
- Auto-reconnection logic
- Per-camera debug logging
- WinCC Unified integration support

🚧 **In Progress**:
- Real RTSP integration with retina crate
- H.264 to JPEG transcoding
- FFmpeg integration for advanced transcoding

## Usage

### Starting the Server

```bash
# Run with default configuration
cargo run

# Build optimized release version
cargo build --release
./target/release/rtsp-streaming-server
```

### Testing with Real RTSP Streams

1. Update `config.toml` with your camera details (see Configuration section)

2. Common RTSP URLs:
   - IP Camera: `rtsp://admin:password@192.168.1.100:554/stream`
   - Test stream: `rtsp://wowzaec2demo.streamlock.net/vod/mp4:BigBuckBunny_115k.mov`

### Viewing the Streams

Open your browser to the specific camera path:
- `http://localhost:8080/cam1` - Camera 1
- `http://localhost:8080/cam2` - Camera 2
- `http://localhost:8080/cam3` - Camera 3

The interface shows:
- Live video feed
- Connection status indicator  
- Real-time FPS counter
- Frame count
- Processing latency
- Fullscreen toggle

## Monitoring and Debugging

With debug logging enabled, you'll see camera-specific messages:

```
[cam1] CAPTURE: 30/s Target: 30/s
[cam2] SENDING: 10/s Pings: 0/s
[cam3] ✅ Successfully connected to RTSP server!
[cam1] Available streams: 1
[cam2] RTSP connection error: connection refused
```

This makes it easy to identify issues with specific cameras.

## Performance

Current implementation characteristics:

- **Latency**: ~100-200ms per camera
- **CPU Usage**: Low (multi-threaded, one thread per camera)
- **Memory Usage**: ~50MB base + ~10MB per camera + ~10MB per client
- **Concurrent Cameras**: Limited by system resources
- **Concurrent Clients**: Multiple viewers per camera supported

## Development

### Building

```bash
# Check for compilation errors
cargo check

# Run with debug logging
RUST_LOG=debug cargo run

# Build optimized release
cargo build --release
```

### Project Structure

```
rtsp-streaming-server/
├── src/
│   ├── main.rs          # Entry point and server setup
│   ├── config.rs        # Configuration management
│   ├── video_stream.rs  # Video stream manager
│   ├── rtsp_client.rs   # RTSP connection handling
│   ├── transcoder.rs    # Video transcoding logic
│   └── websocket.rs     # WebSocket server
├── static/
│   └── index.html       # Web interface
├── config.toml          # Configuration file
└── Cargo.toml           # Dependencies
```

### Dependencies

- **tokio**: Async runtime
- **axum**: Web server framework
- **retina**: RTSP client library
- **image**: JPEG encoding
- **tokio-tungstenite**: WebSocket implementation

## Next Steps

To make this production-ready, consider implementing:

1. **Dashboard View**: Single page showing all cameras
2. **H.264 Processing**: Direct H.264 to browser streaming
3. **FFmpeg Integration**: Advanced transcoding options
4. **WebRTC Support**: Ultra-low latency streaming
5. **Authentication**: User management and access control
6. **Recording**: Save streams to disk
7. **Motion Detection**: Alert on motion events
8. **Mobile Support**: Responsive design and mobile app
9. **Camera Management API**: Add/remove cameras dynamically
10. **Health Monitoring**: Camera status and diagnostics endpoint

## License

This project is open source. Feel free to modify and use as needed.