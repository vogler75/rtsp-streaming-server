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
   # Dashboard (overview of all cameras)
   http://localhost:8080/
   
   # Individual camera test pages
   http://localhost:8080/cam1
   http://localhost:8080/cam2
   http://localhost:8080/cam3
   ```
   
   For HTTPS (if TLS is enabled):
   ```
   https://localhost:8080/
   https://localhost:8080/cam1
   ```

3. **Configure cameras** by editing `config.toml` (see Configuration section)

## URL Structure

The server provides different endpoints for various functionality:

### Main Endpoints
- **`/`** - Dashboard (overview of all cameras with status, controls, and monitoring)
- **`/dashboard`** - Alternative route to dashboard

### Camera Endpoints
For each camera configured with `path = "/cam1"`:

- **`/cam1`** - Camera test page (also serves WebSocket connection for streaming)
- **`/cam1/test`** - Explicit camera test page 
- **`/cam1/stream`** - Video streaming page (WebSocket streaming interface)
- **`/cam1/control`** - Camera control interface (recording, playback, live streaming)

### CWC Integration

For **Siemens WinCC Unified (CWC)** integration, use the appropriate streaming endpoint:

- **Simple Streaming**: Use `/<cam-name>/stream` as the URL in CWC
  ```
  https://your-server/cam1/stream
  https://your-server/cam2/stream
  ```

- **Control Interface**: If you need recording/playback controls in CWC, use `/<cam-name>/control` as the URL
  ```
  https://your-server/cam1/control
  https://your-server/cam2/control
  ```

The **`/stream`** endpoint provides a clean video streaming interface optimized for embedding in CWC, while **`/control`** provides full recording and playback functionality.

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
- **max_recording_age**: Override max recording age for this camera (e.g., "10m", "5h", "7d")

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
   # For simple streaming (recommended for CWC):
   https://your-wincc-server/video/cam1/stream
   https://your-wincc-server/video/cam2/stream
   https://your-wincc-server/video/cam3/stream
   
   # For control interface (recording/playback):
   https://your-wincc-server/video/cam1/control
   https://your-wincc-server/video/cam2/control
   https://your-wincc-server/video/cam3/control
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

Open your browser to access different interfaces:

**Dashboard (recommended starting point):**
- `http://localhost:8080/` - Overview of all cameras with status and controls

**Individual Camera Pages:**
- `http://localhost:8080/cam1` - Camera 1 test page (WebSocket streaming)
- `http://localhost:8080/cam1/stream` - Camera 1 streaming interface  
- `http://localhost:8080/cam1/control` - Camera 1 control interface

**Features by Interface:**

- **Dashboard**: Server status, camera tiles, recording controls, database sizes
- **Test pages** (`/cam1`, `/cam1/test`): Live video feed, connection status, FPS counter, frame count, processing latency, fullscreen toggle
- **Stream pages** (`/cam1/stream`): Clean video streaming interface optimized for embedding
- **Control pages** (`/cam1/control`): Full recording controls, playback, timeline navigation, live streaming

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

## Recording System

The server includes a comprehensive recording system that stores camera streams to a SQLite database for later playback.

### Recording Features

- **Manual Recording**: Start/stop recording via REST API or control interface
- **Frame Storage**: Stores individual JPEG frames with timestamps
- **Playback**: Replay recorded footage at variable speeds
- **Time-based Filtering**: Query recordings by date/time ranges
- **Automatic Cleanup**: Periodically delete old recordings to manage disk space

### Recording Configuration

```toml
[recording]
enabled = true
database_path = "recordings.db"
max_frame_size = 10485760  # 10MB max frame size
max_recording_age = "7d"    # Delete recordings older than 7 days
cleanup_interval_hours = 1   # Run cleanup every hour
```

#### Recording Options
- **enabled**: Enable/disable recording system
- **database_path**: SQLite database file location
- **max_frame_size**: Maximum size for a single frame in bytes
- **max_recording_age**: Maximum age for recordings before deletion
  - Format: `"10m"` (minutes), `"5h"` (hours), `"7d"` (days)
  - Set to `"0"` or omit to disable automatic cleanup
- **cleanup_interval_hours**: How often to run the cleanup task (default: 1 hour)

### Per-Camera Recording Settings

You can override the global `max_recording_age` for individual cameras:

```toml
[cameras.cam1]
max_recording_age = "1d"  # Keep cam1 recordings for only 1 day

[cameras.cam2]
max_recording_age = "30d" # Keep cam2 recordings for 30 days

[cameras.cam3]
# Uses global max_recording_age setting
```

### Automatic Cleanup

When `max_recording_age` is configured, the server will:
1. Start a background cleanup task that runs every `cleanup_interval_hours`
2. Delete frames older than the specified age based on their timestamp
3. Delete completed recording sessions that ended before the cutoff time
4. Preserve active/ongoing recordings (sessions without end_time)
5. Process each camera independently based on its configuration
6. Log cleanup activities for monitoring

The cleanup process:
- Runs in a transaction to ensure database consistency
- Deletes old frames by timestamp (not by session)
- Only deletes completed sessions (where end_time < cutoff)
- Keeps active recordings intact, even if they started long ago
- Cleans up orphaned sessions with no remaining frames
- Handles per-camera overrides
- Reports number of deleted sessions and frames

This design ensures that continuous recordings are preserved while old data is cleaned up efficiently.

## Control API

The server provides both WebSocket and HTTP REST APIs for controlling cameras. Recording management uses REST API, while replay and live streaming use WebSocket.

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

Send JSON commands to control playback and live streaming:

##### Start Replay
```json
{
  "cmd": "start",
  "from": "2025-08-15T10:00:00.000Z",
  "to": "2025-08-15T11:00:00.000Z"  // Optional - if omitted, plays until end
}
```

##### Stop (Replay or Live Stream)
```json
{
  "cmd": "stop"
}
```

##### Adjust Replay Speed
```json
{
  "cmd": "speed",
  "speed": 2.0
}
```

##### Start Live Stream
```json
{
  "cmd": "live"
}
```

##### Go To Timestamp
```json
{
  "cmd": "goto",
  "timestamp": "2025-08-15T10:30:00.000Z"
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
- **Video frames** (type `0x00`): JPEG frame data with timestamp for live streams and replay
  - Format: `[0x00][8-byte timestamp][JPEG data]`
  - Timestamp: Little-endian 64-bit integer (milliseconds since epoch)
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
- **Advanced Replay Controls**: Enhanced playback with:
  - Timeline slider for seeking
  - Frame-by-frame navigation with timestamps
  - Rewind/forward buttons (5-second jumps)
  - Play from current position
  - Variable speed control
  - Real-time timestamp display
- **Recording List**: Browse and select from available recordings

#### Enhanced Video Controls

- **Timeline Scrubbing**: Interactive slider to seek to any point in a recording
- **Timestamp Navigation**: Go to specific timestamps with frame-perfect accuracy
- **Smart Seeking**: Automatically finds the nearest available frame when seeking
- **Continuous Playback**: Play from any point to the end of recordings
- **Real-time Feedback**: Live timestamp display during playback

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