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
   http://localhost:8080/dashboard
   
   # Individual camera test pages
   http://localhost:8080/cam1
   http://localhost:8080/cam2
   http://localhost:8080/cam3
   ```
   
   For HTTPS (if TLS is enabled):
   ```
   https://localhost:8080/dashboard
   https://localhost:8080/cam1
   ```

3. **Configure cameras** by creating JSON files in the `cameras` directory (see Configuration section)

## URL Structure

The server provides different endpoints for various functionality:

### Main Endpoints
- **`/dashboard`** - Dashboard (overview of all cameras with status, controls, and monitoring)

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
### PTZ (Pan/Tilt/Zoom)

Enable PTZ per camera if supported (currently ONVIF):

```json
{
  "path": "/cam1",
  "url": "rtsp://...",
  "transport": "tcp",
  "reconnect_interval": 10,
  "ptz": {
    "enabled": true,
    "protocol": "onvif",
    "onvif_url": "http://<ip>:<port>/onvif/device_service",
    "username": "admin",
    "password": "pass",
    "profile_token": "profile1"
  }
}
```

If `profile_token` is omitted, `profile1` is used. The service URL may vary by device.

The server uses two configuration methods:
1. **`config.json`**: Main server configuration (server settings, MQTT, transcoding defaults, recording)
2. **`cameras/` directory**: Individual camera configurations as JSON files

### Complete Configuration Example

Here's a complete `config.json` file with all sections:

```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 8080,
    "cors_allow_origin": "*",
    "admin_token": "your-secure-admin-token",
    "tls": {
      "enabled": false,
      "cert_path": "certs/server.crt",
      "key_path": "certs/server.key"
    }
  },
  "mqtt": {
    "enabled": true,
    "broker_url": "mqtt://192.168.1.4:1883",
    "client_id": "videoserver-01",
    "base_topic": "Videoserver",
    "qos": 0,
    "retain": false,
    "keep_alive_secs": 60,
    "publish_interval_secs": 1,
    "publish_picture_arrival": true,
    "max_packet_size": 268435456
  },
  "recording": {
    "enabled": true,
    "database_path": "recordings",
    "max_frame_size": 10485760,
    "max_recording_age": "7d",
    "cleanup_interval_hours": 1
  },
  "transcoding": {
    "output_format": "mjpeg",
    "capture_framerate": 5,
    "output_framerate": 0,
    "channel_buffer_size": 50,
    "debug_capture": false,
    "debug_duplicate_frames": false
  }
}
```

### Server Configuration

```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 8080,
    "cors_allow_origin": "*",
    "admin_token": "your-secure-admin-token",
    "tls": {
      "enabled": false,
      "cert_path": "certs/server.crt",
      "key_path": "certs/server.key"
    }
  }
}
```

### MQTT Configuration (Optional)

```json
{
  "mqtt": {
    "enabled": true,
    "broker_url": "mqtt://192.168.1.4:1883",
    "client_id": "videoserver-01",
    "username": "mqtt_user",
    "password": "mqtt_pass",
    "base_topic": "Videoserver",
    "qos": 0,
    "retain": false,
    "keep_alive_secs": 60,
    "publish_interval_secs": 5,
    "publish_picture_arrival": true,
    "max_packet_size": 268435456
  }
}
```

### Global Transcoding Settings

```json
{
  "transcoding": {
    "output_format": "mjpeg",
    "quality": 50,
    "capture_framerate": 0,
    "output_framerate": 5,
    "channel_buffer_size": 1,
    "debug_capture": true,
    "debug_duplicate_frames": false
  }
}
```

### Camera Configuration

Cameras are configured using individual JSON files in the `cameras/` directory. Each file represents one camera configuration. The server automatically detects changes to these files and can add, update, or remove cameras without requiring a restart.

#### Camera Directory Structure
```
cameras/
├── cam1.json
├── cam2.json
├── cam3.json
└── cam4.json
```

#### Camera Configuration File Format

Create a JSON file for each camera in the `cameras/` directory:

**Example: `cameras/cam1.json`**
```json
{
  "enabled": true,
  "path": "/cam1",
  "url": "rtsp://admin007:admin007@192.168.1.171:554/stream1",
  "transport": "tcp",
  "reconnect_interval": 5,
  "chunk_read_size": 8192,
  "token": "secure-cam1-token",
  "max_recording_age": "7d",
  
  "ffmpeg": {
    "command": null,
    "use_wallclock_as_timestamps": true,
    "output_format": "mjpeg",
    "video_codec": "mjpeg",
    "video_bitrate": "200k",
    "quality": 75,
    "output_framerate": 5,
    "scale": "640:-1",
    "movflags": null,
    "rtbufsize": 65536,
    "fflags": "+nobuffer+discardcorrupt",
    "flags": "low_delay",
    "avioflags": "direct",
    "fps_mode": "cfr",
    "flush_packets": "1",
    "extra_input_args": ["-analyzeduration", "100000", "-probesize", "100000"],
    "extra_output_args": [],
    "log_stderr": "file"
  },
  
  "mqtt": {
    "publish_interval": 5,
    "topic_name": "surveillance/cameras/cam1/image"
  },
  
  "transcoding_override": {
    "output_format": "mjpeg",
    "quality": 75,
    "capture_framerate": 10,
    "output_framerate": 5,
    "channel_buffer_size": 50,
    "debug_capture": false,
    "debug_duplicate_frames": false
  }
}
```

#### Camera Configuration Options

##### Basic Settings
- **`enabled`** (boolean): Enable/disable this camera (default: `true`)
- **`path`** (string): URL path for this camera (e.g., `"/cam1"`)
- **`url`** (string): RTSP/HTTP camera URL with credentials
  - RTSP: `"rtsp://user:pass@192.168.1.100:554/stream1"`
  - HTTP: `"http://user:pass@192.168.1.100/video.mjpg"`
  - Note: URL-encode special characters in passwords (e.g., `#` → `%23`)
- **`transport`** (string): Transport protocol - `"tcp"` or `"udp"` (for RTSP only)
- **`reconnect_interval`** (number): Seconds between reconnection attempts (default: `5`)
- **`chunk_read_size`** (number|null): Bytes to read at once from FFmpeg
- **`token`** (string|null): Optional token required for WebSocket authentication
- **`frame_storage_retention`** (string|null): Override max recording age (e.g., `"10m"`, `"5h"`, `"7d"`)

##### FFmpeg Settings (`ffmpeg` object)
- **`command`** (string|null): Custom FFmpeg command override. If set, replaces all other FFmpeg options
  - Placeholder: `$url` will be replaced with the camera URL
  - Example: `"-use_wallclock_as_timestamps 1 -rtsp_transport tcp -i $url -an -c:v mjpeg -q:v 4 -vf fps=15 -f mjpeg -"`
- **`use_wallclock_as_timestamps`** (boolean|null): Add `-use_wallclock_as_timestamps 1` as first option (default: `true`)
- **`output_format`** (string|null): Output format, typically `"mjpeg"` for streaming
- **`video_codec`** (string|null): Video codec (e.g., `"mjpeg"`)
- **`video_bitrate`** (string|null): Video bitrate (e.g., `"200k"`, `"1M"`, `"2000k"`)
- **`quality`** (number|null): JPEG quality for MJPEG (1-100, default: `75`)
- **`output_framerate`** (number|null): Output framerate in FPS
- **`scale`** (string|null): Video scaling (e.g., `"640:480"`, `"1280:-1"` for aspect ratio preservation)
- **`movflags`** (string|null): MOV flags for MP4/MOV formats
- **`rtbufsize`** (number|null): RTSP buffer size in bytes (helps with network jitter)
- **`fflags`** (string|null): Format flags (e.g., `"+nobuffer+discardcorrupt"` for low latency)
- **`flags`** (string|null): Codec flags (e.g., `"low_delay"`)
- **`avioflags`** (string|null): AVIO flags (e.g., `"direct"` to bypass buffering)
- **`fps_mode`** (string|null): Frame rate mode - `"cfr"` (constant), `"vfr"` (variable), `"passthrough"`
- **`flush_packets`** (string|null): Flush packets immediately (`"1"` for low latency)
- **`extra_input_args`** (array|null): Additional FFmpeg input arguments
- **`extra_output_args`** (array|null): Additional FFmpeg output arguments
- **`log_stderr`** (string|null): FFmpeg stderr logging - `"file"`, `"console"`, `"both"`, or `null` to disable

##### MQTT Settings (`mqtt` object)
Camera-specific MQTT settings (optional):
- **`publish_interval`** (number): Seconds between MQTT image publishes (0 = every frame)
- **`topic_name`** (string): MQTT topic for camera images

##### Transcoding Override (`transcoding_override` object)
Override global transcoding settings for this camera (optional):
- **`output_format`** (string): Output format (e.g., `"mjpeg"`)
- **`quality`** (number): JPEG quality (1-100)
- **`capture_framerate`** (number): Capture rate from camera (0 = max available)
- **`output_framerate`** (number): Output framerate
- **`channel_buffer_size`** (number): Frame buffer size
- **`debug_capture`** (boolean): Enable capture rate debug output
- **`debug_duplicate_frames`** (boolean): Enable duplicate frame detection logging

#### Dynamic Camera Management

The server watches the `cameras/` directory for changes and automatically:
- **Adds** new cameras when JSON files are created
- **Updates** camera settings when files are modified
- **Removes** cameras when files are deleted

All changes are applied without server restart. WebSocket connections and FFmpeg processes are properly managed during these operations.

#### Example Camera Configurations

**Minimal Configuration (`cameras/simple.json`):**
```json
{
  "enabled": true,
  "path": "/simple",
  "url": "rtsp://192.168.1.100:554/stream1",
  "transport": "tcp",
  "reconnect_interval": 5
}
```

**High-Quality Camera (`cameras/hq_cam.json`):**
```json
{
  "enabled": true,
  "path": "/hq_cam",
  "url": "rtsp://admin:pass@192.168.1.200:554/h264",
  "transport": "tcp",
  "reconnect_interval": 5,
  "ffmpeg": {
    "quality": 95,
    "scale": "1920:-1",
    "output_framerate": 30,
    "video_bitrate": "5M"
  }
}
```

**Low-Latency Camera (`cameras/fast_cam.json`):**
```json
{
  "enabled": true,
  "path": "/fast_cam",
  "url": "rtsp://192.168.1.150:554/live",
  "transport": "udp",
  "reconnect_interval": 2,
  "ffmpeg": {
    "rtbufsize": 32768,
    "fflags": "+nobuffer+discardcorrupt",
    "flags": "low_delay",
    "avioflags": "direct",
    "flush_packets": "1",
    "extra_input_args": ["-analyzeduration", "100000", "-probesize", "100000"]
  }
}
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

Once you have certificates, enable HTTPS in your `config.json`:

```json
{
  "server": {
    "tls": {
      "enabled": true,
      "cert_path": "certs/server.crt",
      "key_path": "certs/server.key"
    }
  }
}
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

## Admin Interface

The server includes a web-based admin interface for managing camera configurations without editing JSON files directly.

### Accessing the Admin Interface

Navigate to:
```
http://localhost:8080/admin
```

### Features

- **Camera Management**: Add, edit, and delete camera configurations
- **Live Configuration**: All changes are applied immediately without server restart
- **Visual Configuration**: User-friendly forms for all camera settings
- **Configuration Groups**: Settings organized into logical sections:
  - Basic Settings (URL, path, transport)
  - MQTT Settings (publishing intervals, topics)
  - FFmpeg Settings (quality, scaling, performance)
  - Extended Options (advanced FFmpeg parameters)

### Using the Admin Interface

1. **View Cameras**: See all configured cameras with their status
2. **Add Camera**: Click "Add Camera" and fill in the required fields
3. **Edit Camera**: Click the edit icon next to any camera to modify settings
4. **Delete Camera**: Click the delete icon to remove a camera
5. **Save Changes**: Click "Save Camera" to apply configuration changes

All changes made through the admin interface are saved to JSON files in the `cameras/` directory and are immediately applied by the server's file watcher.

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

1. Update `config.json` with your camera details (see Configuration section)

2. Common RTSP URLs:
   - IP Camera: `rtsp://admin:password@192.168.1.100:554/stream`
   - Test stream: `rtsp://wowzaec2demo.streamlock.net/vod/mp4:BigBuckBunny_115k.mov`

### Viewing the Streams

Open your browser to access different interfaces:

**Dashboard (recommended starting point):**
- `http://localhost:8080/dashboard` - Overview of all cameras with status and controls

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

The server includes a comprehensive dual-format recording system that provides both granular frame-by-frame access and efficient video segment storage.

### Recording Features

- **Dual Storage System**: 
  - Frame-by-frame SQLite database for precise playback control
  - MP4 video segments for efficient long-term storage
- **Manual Recording**: Start/stop recording via REST API or control interface
- **Frame Storage**: Stores individual JPEG frames with timestamps for precise seeking
- **Video Segments**: Creates time-based MP4 files for efficient storage and playback
- **Playback**: Replay recorded footage at variable speeds from either storage format
- **Time-based Filtering**: Query recordings by date/time ranges
- **Automatic Cleanup**: Independent retention policies for frames vs. video segments

### Recording Configuration

The recording system supports two storage formats with independent configuration:

```json
{
  "recording": {
    "frame_storage_enabled": true,
    "video_storage_enabled": true,
    "database_path": "recordings",
    "max_frame_size": 10485760,
    "frame_storage_retention": "7d",
    "video_storage_retention": "30d",
    "video_segment_minutes": 5,
    "cleanup_interval_hours": 1
  }
}
```

Or using TOML format:

```toml
[recording]
# Frame-by-frame recording for high-granularity, short-term playback
frame_storage_enabled = true
database_path = "recordings"
max_frame_size = 10485760
frame_storage_retention = "7d"  # Delete frame recordings older than this

# MP4 video segment recording for efficient, long-term storage  
video_storage_enabled = true
video_storage_retention = "30d"  # Delete video segments older than this
video_segment_minutes = 5        # Duration of each MP4 video segment in minutes

# Global cleanup settings
cleanup_interval_hours = 1  # How often to run the cleanup process
```

#### Recording Options
- **frame_storage_enabled**: Enable/disable frame-by-frame SQLite storage
- **video_storage_enabled**: Enable/disable MP4 video segment creation
- **database_path**: Base path for database files and video segments
- **max_frame_size**: Maximum size for a single frame in bytes (SQLite storage)
- **frame_storage_retention**: Maximum age for frame recordings before deletion
  - Format: `"10m"` (minutes), `"5h"` (hours), `"7d"` (days)
  - Typically shorter for high-granularity access (e.g., 1-7 days)
- **video_storage_retention**: Maximum age for video segments before deletion
  - Format: `"10m"` (minutes), `"5h"` (hours), `"30d"` (days)
  - Typically longer for archival storage (e.g., 30-90 days)
- **video_segment_minutes**: Duration of each MP4 video segment in minutes (default: 5)
- **cleanup_interval_hours**: How often to run the cleanup task (default: 1 hour)

### Per-Camera Recording Settings

You can override global recording settings for individual cameras in their configuration files:

```json
{
  "path": "/cam1",
  "url": "rtsp://...",
  "frame_storage_retention": "1d",
  "video_storage_retention": "14d"
}
```

### Automatic Cleanup

The server runs independent cleanup processes for both storage formats:

#### Frame Storage Cleanup
When `frame_storage_retention` is configured:
1. Background cleanup task runs every `cleanup_interval_hours`
2. Deletes frames older than the retention period based on timestamp
3. Deletes completed recording sessions that ended before the cutoff time
4. Preserves active/ongoing recordings (sessions without end_time)
5. Processes each camera independently based on its configuration

#### Video Storage Cleanup
When `video_storage_retention` is configured:
1. Scans MP4 files in the recordings directory
2. Deletes video segments older than the retention period based on filename timestamp
3. Uses hierarchical directory structure for organization (YYYY/MM/DD)
4. Processes each camera's video files independently

#### Cleanup Process Details
- Runs in transactions to ensure database consistency (frame storage)
- Deletes old data by timestamp, not by recording session
- Preserves active recordings even if they started long ago
- Handles per-camera retention overrides
- Logs cleanup activities for monitoring
- Reports number of deleted sessions, frames, and video files

This dual-format design allows you to maintain short-term high-granularity access (frames) while preserving long-term efficient storage (video segments).

### File Structure

The recording system creates the following directory structure:

```
recordings/
├── cam1.db                           # Frame-by-frame SQLite database
├── cam1/                             # MP4 video segments directory
│   ├── 2024/
│   │   └── 08/
│   │       └── 19/
│   │           ├── cam1_20240819_140000.mp4    # 5-minute segments
│   │           ├── cam1_20240819_140500.mp4
│   │           └── cam1_20240819_141000.mp4
├── cam2.db                           # Frame database for camera 2
├── cam2/                             # MP4 segments for camera 2
│   └── 2024/
│       └── 08/
│           └── 19/
│               └── cam2_20240819_140000.mp4
```

#### File Naming Convention
- **SQLite databases**: `{camera_id}.db`
- **MP4 segments**: `{camera_id}_{YYYYMMDD}_{HHMMSS}.mp4`
- **Directory structure**: `recordings/{camera_id}/{YYYY}/{MM}/{DD}/`

This hierarchical structure makes it easy to navigate recordings by date and enables efficient cleanup of old video segments.

### How the Dual Storage System Works

The recording system uses a unified architecture where both storage formats receive frames from the same source:

```
RTSP Camera → FFmpeg (MJPEG) → Broadcast Channel → {
  ├── WebSocket clients (live streaming)
  ├── Frame recorder → SQLite database (frame-by-frame storage)
  └── Video segmenter → Buffer → FFmpeg (MP4) → MP4 files
}
```

#### Frame Storage (SQLite)
- Stores individual JPEG frames with precise timestamps
- Enables frame-by-frame playback and seeking
- Ideal for short-term storage with high granularity
- Used for precise timeline scrubbing and frame analysis

#### Video Storage (MP4)
- Collects frames in a buffer for the configured segment duration
- Creates MP4 files using FFmpeg with H.264 encoding
- Provides efficient compression for long-term storage
- Suitable for continuous playback and archival purposes

Both systems operate independently and can be enabled/disabled separately. You can configure different retention policies - for example, keep frames for 1 day for precise analysis and video segments for 30 days for long-term review.

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