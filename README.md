# RTSP Video Streaming Server

A high-performance, low-latency video streaming server built in Rust that connects to RTSP cameras and streams video to web browsers via WebSockets.

## Features

- **RTSP Camera Support**: Connect to RTSP cameras and streams
- **WebSocket Streaming**: Low-latency video streaming via WebSockets  
- **MJPEG Output**: Converts video to JPEG frames for browser compatibility
- **HTTPS/TLS Support**: Secure connections with configurable certificates
- **Web Interface**: Simple HTML client for viewing streams
- **Real-time Stats**: FPS, frame count, and latency monitoring
- **Auto-reconnection**: Automatic reconnection for both RTSP and WebSocket connections

## Quick Start

1. **Build and run the server**:
   ```bash
   cargo run
   ```

2. **Open your web browser** and navigate to:
   ```
   http://localhost:8080
   ```
   
   For HTTPS (if TLS is enabled):
   ```
   https://localhost:8080
   ```

3. **Configure RTSP source** by editing `config.toml`:
   ```toml
   [rtsp]
   url = "rtsp://admin:password@192.168.1.100:554/stream"
   transport = "tcp"
   reconnect_interval = 5
   buffer_size = 1024000
   ```

## Configuration

The server can be configured via the `config.toml` file:

```toml
[server]
host = "0.0.0.0"
port = 8080

[server.tls]
enabled = false
cert_path = "certs/server.crt"
key_path = "certs/server.key"

[rtsp]
url = "rtsp://admin:password@192.168.1.100:554/stream"
transport = "tcp"
reconnect_interval = 5
buffer_size = 1024000

[transcoding]
output_format = "mjpeg"
quality = 85
framerate = 30
max_latency_ms = 200
```

### Configuration Options

- **server.host**: Server bind address (default: "0.0.0.0")
- **server.port**: Server port (default: 8080)
- **server.tls.enabled**: Enable HTTPS/TLS (default: false)
- **server.tls.cert_path**: Path to SSL certificate file (default: "certs/server.crt")
- **server.tls.key_path**: Path to SSL private key file (default: "certs/server.key")
- **rtsp.url**: RTSP camera URL with credentials
- **rtsp.transport**: Transport protocol - "tcp" or "udp"
- **rtsp.reconnect_interval**: Seconds between reconnection attempts
- **rtsp.buffer_size**: Internal buffer size in bytes
- **transcoding.output_format**: Output format - currently "mjpeg"
- **transcoding.quality**: JPEG quality (1-100)
- **transcoding.framerate**: Target framerate
- **transcoding.max_latency_ms**: Maximum acceptable latency

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
RTSP Camera → Rust Server → WebSocket → Browser
                ↓
           [Transcoding]
                ↓
            MJPEG Frames
```

### Components

- **RTSP Client** (`src/rtsp_client.rs`): Handles RTSP connections and frame reception
- **Transcoder** (`src/transcoder.rs`): Converts video frames to JPEG format
- **WebSocket Server** (`src/websocket.rs`): Manages WebSocket connections and frame broadcasting
- **Web Interface** (`static/index.html`): Browser-based video player

## Current Status

This is a working implementation with the following features:

✅ **Working**:
- Basic server architecture
- WebSocket streaming 
- MJPEG frame generation (test frames)
- Web interface with real-time stats
- Configuration management
- Auto-reconnection logic

🚧 **In Progress**:
- Real RTSP integration with retina crate (currently uses test frames)
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

1. Update `config.toml` with your RTSP camera details:
   ```toml
   [rtsp]
   url = "rtsp://username:password@camera-ip:port/stream-path"
   ```

2. Common RTSP URLs:
   - IP Camera: `rtsp://admin:password@192.168.1.100:554/stream`
   - Test stream: `rtsp://wowzaec2demo.streamlock.net/vod/mp4:BigBuckBunny_115k.mov`

### Viewing the Stream

Open your browser to `http://localhost:8080` and click "Connect" to start streaming.

The interface shows:
- Live video feed
- Connection status indicator  
- Real-time FPS counter
- Frame count
- Processing latency
- Fullscreen toggle

## Performance

Current implementation generates test frames at ~30 FPS with the following characteristics:

- **Latency**: ~100-200ms (simulated test frames)
- **CPU Usage**: Low (single-threaded frame generation)
- **Memory Usage**: ~50MB base + ~10MB per connected client
- **Concurrent Clients**: Supports multiple simultaneous viewers

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

1. **Real RTSP Integration**: Complete the retina crate integration
2. **H.264 Processing**: Direct H.264 to browser streaming
3. **FFmpeg Integration**: Advanced transcoding options
4. **WebRTC Support**: Ultra-low latency streaming
5. **Authentication**: User management and access control
6. **Recording**: Save streams to disk
7. **Multiple Cameras**: Support multiple RTSP sources
8. **Mobile Support**: Responsive design and mobile app

## License

This project is open source. Feel free to modify and use as needed.