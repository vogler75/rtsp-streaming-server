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

### MQTT Configuration (Optional)

```toml
[mqtt]
enabled = true
broker_url = "mqtt://192.168.1.4:1883"  # Can also use mqtts:// for TLS
client_id = "videoserver-01"
# username = "mqtt_user"  # Optional
# password = "mqtt_pass"  # Optional
base_topic = "Videoserver"  # Base topic for all MQTT messages
qos = 0  # Quality of Service (0, 1, or 2)
retain = false  # Whether to retain messages
keep_alive_secs = 60  # Keep-alive interval in seconds
publish_interval_secs = 5  # How often to publish status updates
publish_picture_arrival = true  # Enable/disable picture arrival publishing (default: true)
```

### Global Transcoding Settings

```toml
[transcoding]
output_format = "mjpeg"
quality = 50
capture_framerate = 0      # 0 = max available from camera
output_framerate = 5       # Output FPS (can be overridden per camera)
channel_buffer_size = 1    # Frame buffer size (1 = only latest)
debug_capture = true       # Show capture rate debug info
```

### Multi-Camera Configuration

Configure multiple cameras, each with its own path and settings. The server supports both RTSP and HTTP/HTTPS stream sources:

```toml
# RTSP Camera Example
[cameras.cam1]
enabled = false  # Optional: Enable/disable this camera (default: true)
path = "/cam1"
url = "rtsp://admin007:admin007@192.168.1.171:554/stream1"
transport = "tcp"
reconnect_interval = 5
chunk_read_size = 8192 # 32768
token = "secure-cam1-token"  # Optional: Token required for WebSocket authentication

# FFmpeg configuration for cam1
[cameras.cam1.ffmpeg]
log_stderr = "file"  # FFmpeg stderr logging: "file", "console", "both"

# Command override - if set, replaces all other FFmpeg options with a custom command
# Placeholder: $url = camera RTSP URL
#command = "-use_wallclock_as_timestamps 1 -rtsp_transport tcp -i $url -an -c:v mjpeg -q:v 4 -vf fps=15 -f mjpeg -"

use_wallclock_as_timestamps = true  # Adds "-use_wallclock_as_timestamps 1" as first option

scale = "640:-1"  # Video scaling: "640:480", "1280:-1", etc.

#output_format = "mjpeg"  # Output format: MJPEG for streaming
#output_framerate = 5  # Output framerate (fps)

#video_codec = "mjpeg"  # Video codec: mjpeg is default for MJPEG format
#video_bitrate = "200k"  # Video bitrate: "200k", "1M", "2000k", etc.

# FFmpeg advanced options for low latency
#rtbufsize = 65536  # RTSP buffer size in bytes
#fflags = "+nobuffer+discardcorrupt"
#flags = "low_delay"
#avioflags = "direct"
#fps_mode = "cfr"  # Use "cfr" for constant framerate, "vfr" for variable, "passthrough" to keep original
#flush_packets = "1"
#extra_input_args = [] # ["-analyzeduration", "100000", "-probesize", "100000"]
#extra_output_args = []

# HTTP/ASF Stream Example
[cameras.cam4]
enabled = true  # Optional: Enable/disable this camera (default: true)
path = "/cam4"
# For HTTP streams with authentication, use Basic Auth format:
# Note: Special characters in password must be URL-encoded (e.g., # = %23)
url = "http://Admin007:Admin007%23%23@outdoor2:80/videostream.asf?resolution=64&rate=0"
transport = "tcp"  # Ignored for HTTP streams
reconnect_interval = 5

[cameras.cam4.ffmpeg]
log_stderr = "file"  # FFmpeg stderr logging: "file", "console", "both"
scale = "640:-1"  # Video scaling: "640:480", "1280:-1", etc.
```

### Configuration Options

#### Server Options
- **server.host**: Server bind address (default: "0.0.0.0")
- **server.port**: Server port (default: 8080)
- **server.cors_allow_origin**: CORS allowed origin (default: "*")
- **server.tls.enabled**: Enable HTTPS/TLS (default: false)
- **server.tls.cert_path**: Path to SSL certificate file
- **server.tls.key_path**: Path to SSL private key file

#### MQTT Options
- **mqtt.enabled**: Enable/disable MQTT publishing (default: false)
- **mqtt.broker_url**: MQTT broker URL (mqtt:// or mqtts://)
- **mqtt.client_id**: Unique client identifier
- **mqtt.username**: Optional authentication username
- **mqtt.password**: Optional authentication password
- **mqtt.base_topic**: Base topic prefix for all messages
- **mqtt.qos**: Quality of Service level (0, 1, or 2)
- **mqtt.retain**: Whether to retain messages
- **mqtt.keep_alive_secs**: Keep-alive interval in seconds
- **mqtt.publish_interval_secs**: How often to publish status updates
- **mqtt.publish_picture_arrival**: Enable/disable picture arrival events (default: true)

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
- **output_framerate**: Output framerate (can be overridden per camera)
- **channel_buffer_size**: Number of frames to buffer
- **debug_capture**: Enable capture rate debug output

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

## Control API

The server provides both WebSocket and HTTP REST APIs for controlling camera recordings and playback.

### WebSocket Control API

Each camera has a control WebSocket endpoint at `/<camera_path>/control` that provides real-time control functionality.

#### Connection

```javascript
const ws = new WebSocket('ws://localhost:8080/cam1/control');
// For cameras with token authentication:
const ws = new WebSocket('ws://localhost:8080/cam1/control?token=your-token');
```

#### Authentication

For cameras configured with tokens, include the token as a query parameter or send an `Authorization: Bearer <token>` header.

#### WebSocket Commands

Send JSON commands to control recording, playback, and live streaming:

##### Start Recording
```json
{
  "cmd": "startrecording",
  "reason": "Security event detected"
}
```

##### Stop Recording
```json
{
  "cmd": "stoprecording"
}
```

##### List Recordings
```json
{
  "cmd": "listrecordings",
  "from": "2025-08-15T00:00:00.000Z",
  "to": "2025-08-15T23:59:59.999Z"
}
```

##### Start Replay
```json
{
  "cmd": "startreplay",
  "from": "2025-08-15T10:00:00.000Z",
  "to": "2025-08-15T11:00:00.000Z"
}
```

##### Stop Replay
```json
{
  "cmd": "stopreplay"
}
```

##### Adjust Replay Speed
```json
{
  "cmd": "replayspeed",
  "speed": 2.0
}
```

##### Start Live Stream
```json
{
  "cmd": "startlivestream"
}
```

##### Stop Live Stream
```json
{
  "cmd": "stoplivestream"
}
```

#### WebSocket Responses

All commands return JSON responses:

```json
{
  "code": 200,
  "text": "Recording started (session 123)",
  "data": {
    "session_id": 123
  }
}
```

Error responses:
```json
{
  "code": 404,
  "text": "No active recording found"
}
```

#### Binary Data

The WebSocket connection also receives binary data:
- **Video frames** (type `0x00`): JPEG frame data for live streams and replay
- **JSON responses** (type `0x01`): Command responses and status updates

### HTTP REST API

HTTP endpoints provide programmatic access to recording management functionality.

#### Authentication

For cameras with token authentication, include the token in the Authorization header:
```
Authorization: Bearer your-token-here
```

#### Endpoints

##### Start Recording
```http
POST /<camera_path>/control/recording/start
Content-Type: application/json

{
  "reason": "Security event detected"
}
```

##### Stop Recording
```http
POST /<camera_path>/control/recording/stop
```

##### List Recordings
```http
GET /<camera_path>/control/recordings?from=2025-08-15T00:00:00.000Z&to=2025-08-15T23:59:59.999Z
```

##### Get Recorded Frames
```http
GET /<camera_path>/control/recordings/<session_id>/frames?from=2025-08-15T10:00:00.000Z&to=2025-08-15T11:00:00.000Z
```

##### Get Active Recording
```http
GET /<camera_path>/control/recording/active
```

#### Response Format

All REST API responses follow this format:

**Success Response:**
```json
{
  "status": "success",
  "data": {
    "session_id": 123,
    "message": "Recording started"
  }
}
```

**Error Response:**
```json
{
  "status": "error",
  "error": "No active recording found",
  "code": 404
}
```

### Control Interface

The server includes a web-based control interface accessible at `/<camera_path>/control` (without WebSocket upgrade). This provides:

- **Recording Controls**: Start/stop recording with optional reason
- **Day Selection**: Easy filtering of recordings by date
- **Time Range Selection**: Precise time range for playback
- **Live Streaming**: Start/stop live video streaming to the control interface
- **Replay Controls**: Playback recorded footage with speed control
- **Recording List**: Browse and select from available recordings

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