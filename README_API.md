# REST API Documentation

This document provides a comprehensive overview of all available REST API endpoints for the RTSP streaming server.

## 🎥 Quick Reference - Video Streaming

| Endpoint | Purpose | Format | Parameters |
|----------|---------|---------|------------|
| `{camera_path}/control/recordings/frames/{timestamp}` | Single frame by timestamp | JPEG | `tolerance` |
| `{camera_path}/control/recordings/mp4/segments/{filename}` | Single MP4 recording | MP4 | - |
| `{camera_path}/control/recordings/hls/timerange` | HLS playlist for time range | M3U8 | `t1`, `t2`, `segment_duration` |

**Example:**
```bash
# HLS playlist for a time range
GET /cam1/control/recordings/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z
```

---

## 🏗️ API Endpoint Hierarchy

```text
/
├── dashboard                                 # Dashboard page
├── debug                                     # Debug interface
└── api/
    ├── status                                # Server status
    ├── cameras                               # List cameras
    └── admin/
        ├── cameras/
        │   ├── POST /                        # Create camera
        │   ├── GET /{id}                     # Get camera config
        │   ├── PUT /{id}                     # Update camera config
        │   └── DELETE /{id}                  # Delete camera
        └── config/
            ├── GET /                         # Get server config
            └── PUT /                         # Update server config

# Per-camera routes (using configured camera path, e.g., /cam1)
{camera_path}/
├── /                                         # Camera test page
├── stream                                    # Stream page (WebSocket frames)
├── control                                   # Control page (WebSocket control)
├── live                                      # Live stream over WebSocket
├── snapshot                                  # Current frame as JPEG
├── test                                      # Alternate test page
└── control/
    ├── recording/
    │   ├── POST start                        # Start recording
    │   ├── POST stop                         # Stop recording
    │   ├── GET active                        # Active recording status
    │   └── GET size                          # Recording DB size
    ├── recordings/
    │   ├── GET /                             # List recordings
    │   ├── GET /{session_id}/frames          # Frame metadata
    │   ├── PUT /{session_id}/keep            # Set session keep/protect flag
    │   ├── GET frames/{timestamp}            # Get single frame by timestamp
    │   ├── mp4/
    │   │   ├── GET segments                  # List MP4 segments
    │   │   └── GET segments/{filename}       # Stream single MP4
    │   └── hls/
    │       ├── GET timerange                 # Generate HLS playlist
    │       └── GET segments/{playlist_id}/{segment_name} # Serve HLS segments
    └── ptz/                                  # PTZ controls (if enabled)
        ├── POST move                         # Pan/tilt/zoom
        ├── POST stop                         # Stop movement
        ├── POST goto_preset                  # Move to preset
        └── POST set_preset                   # Create/update preset
```

---

## 📺 Video Streaming

### Overview

The server provides two main ways to access recorded video content:

- **🎬 Individual MP4 Segments**: Direct access to single recording files
- **📺 HLS Time Range Playlists**: Adaptive streaming for time ranges

### Key Features

- **HLS Transcoding**: Multiple MP4 segments transcoded to HLS on-the-fly using FFmpeg
- **Storage Agnostic**: Works with both database and filesystem storage
- **Browser Compatible**: MP4 works with HTML5 `<video>`, HLS works with HLS.js
- **Time Range Queries**: ISO 8601 timestamps for precise time selection
- **Byte Range Support**: HTTP range requests for video seeking

### Single MP4 Recording

**Endpoint:** `GET {camera_path}/control/recordings/mp4/segments/{filename}`

Stream an individual MP4 recording file for playback.

- **Authentication**: Bearer token if camera has token configured
- **Headers**:
  - `Range` (optional): Byte-range requests for seeking (e.g., `bytes=0-1024`)
  - `Authorization` (optional): `Bearer <camera_token>` if camera requires authentication
- **Response**: 
  - `200 OK`: Full video file
  - `206 Partial Content`: When Range header provided
  - `404 Not Found`: Recording not found
  - `401 Unauthorized`: Missing or invalid authentication
  - Headers: `Content-Type: video/mp4`, `Accept-Ranges: bytes`, `Cache-Control: public, max-age=3600`

**Examples:**
```bash
# Stream full MP4 file
GET /cam1/control/recordings/mp4/segments/2025-08-21T05-39-14Z.mp4

# Stream with byte range for seeking and authentication
GET /cam1/control/recordings/mp4/segments/2025-08-21T05-39-14Z.mp4
Range: bytes=1024-2048
Authorization: Bearer your-camera-token
```

### HLS Time Range Playlist

**Endpoint:** `GET {camera_path}/control/recordings/hls/timerange`

Generate an HLS (HTTP Live Streaming) playlist for recordings within a time range.

- **Authentication**: Bearer token if camera has token configured
- **Headers**:
  - `Authorization` (optional): `Bearer <camera_token>` if camera requires authentication
- **Query Parameters**:
  - `t1` (required): Start time in ISO 8601 format
  - `t2` (required): End time in ISO 8601 format  
  - `segment_duration` (optional): Target segment duration in seconds (default: 10)
- **Response**: 
  - `200 OK`: M3U8 playlist content
  - `404 Not Found`: No recordings in time range
  - `401 Unauthorized`: Missing or invalid authentication
  - Headers: `Content-Type: application/vnd.apple.mpegurl`, `Access-Control-Allow-Origin: *`

**Features:**
- Creates MPEG-TS segments from MP4 recordings
- Compatible with HLS.js, Video.js, and native iOS/macOS players
- Supports adaptive bitrate streaming workflows
- Works with both database and filesystem storage

**Examples:**
```bash
# Basic HLS playlist
GET /cam1/control/recordings/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z

# Custom segment duration with authentication
GET /cam1/control/recordings/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z&segment_duration=5
Authorization: Bearer your-camera-token
```

**HTML5 Usage:**
```html
<script src="https://cdn.jsdelivr.net/npm/hls.js@latest"></script>
<video id="video" controls></video>
<script>
  const video = document.getElementById('video');
  const hls = new Hls();
  hls.loadSource('/cam1/control/recordings/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z');
  hls.attachMedia(video);
</script>
```

The playlist will reference segments available at:
```
GET {camera_path}/control/recordings/hls/segments/{playlist_id}/{segment_name}
```

---

## 📸 Live Frame Snapshot

### Get Current Frame

**Endpoint:** `GET /{camera_path}/snapshot`

Get the current live frame from a camera as a JPEG image. This endpoint captures the most recent frame from the live video stream without requiring recording to be active.

- **Authentication**: Bearer token if camera has token configured
- **Headers**:
  - `Authorization` (optional): `Bearer <camera_token>` if camera requires authentication
- **Query Parameters**:
  - `token` (optional): Camera token as query parameter (alternative to Authorization header)
- **Response**: 
  - **Success (200)**: Raw JPEG binary data with headers:
    - `Content-Type: image/jpeg`
    - `Cache-Control: no-cache, no-store, must-revalidate`
    - `Pragma: no-cache`
    - `Expires: 0`
  - **Service Unavailable (503)**: Camera stream not available, closed, or timeout
  - **Unauthorized (401)**: Missing or invalid authentication
  - **Not Found (404)**: Camera not found

**Features:**
- **Live Stream Integration**: Gets frames directly from the current video stream buffer
- **No Recording Required**: Works independently of the recording system
- **Fast Response**: Returns immediately if a recent frame is available, otherwise waits for the next frame
- **Timeout Protection**: 5-second timeout prevents hanging requests
- **Browser Compatible**: Standard JPEG format works with HTML `<img>` tags and all browsers

**Examples:**
```bash
# Get current snapshot
GET /cam1/snapshot

# With query parameter authentication
GET /cam1/snapshot?token=your-camera-token

# With Bearer token authentication
GET /cam1/snapshot
Authorization: Bearer your-camera-token
```

**HTML Usage:**
```html
<!-- Simple image display -->
<img src="/cam1/snapshot" alt="Camera 1 Current Frame" />

<!-- With token authentication -->
<img src="/cam1/snapshot?token=your-camera-token" alt="Camera 1 Current Frame" />

<!-- JavaScript fetch with Bearer token -->
<script>
  fetch('/cam1/snapshot', {
    headers: {
      'Authorization': 'Bearer your-camera-token'
    }
  })
  .then(response => response.blob())
  .then(blob => {
    const imageUrl = URL.createObjectURL(blob);
    document.getElementById('snapshot').src = imageUrl;
  });
</script>
```

**Implementation Notes:**
- Maintains a dedicated storage of the latest frame from each camera's live stream
- Returns immediately with the most recent frame (no waiting required)
- Each camera runs a background task that continuously updates its latest frame storage
- Returns appropriate HTTP status codes for different error conditions
- Includes cache-control headers to prevent browser caching of dynamic content
- Provides instant response times ideal for frequent polling or real-time applications

---

## 🛠️ Camera Management API

All camera management endpoints require admin authentication via `Authorization: Bearer <admin_token>` header.

**Base Path:** `/api/admin/cameras`

### Create Camera

**Endpoint:** `POST /api/admin/cameras`

Creates a new camera configuration. The server automatically detects and starts the camera stream.

**Request Body:**
```json
{
  "camera_id": "new_cam",
  "config": {
    "path": "/new_cam",
    "url": "rtsp://...",
    "transport": "tcp",
    "reconnect_interval": 10,
    "token": "some-secure-token"
  }
}
```

**Response:** Success or error message

### Get Camera Configuration

**Endpoint:** `GET /api/admin/cameras/{id}`

Retrieves the current configuration for a specific camera.

**Response:** Camera configuration object

### Update Camera Configuration

**Endpoint:** `PUT /api/admin/cameras/{id}`

Updates camera configuration. The server detects changes and restarts the camera stream.

**Request Body:** Complete `CameraConfig` JSON object  
**Response:** Success or error message

### Delete Camera

**Endpoint:** `DELETE /api/admin/cameras/{id}`

Deletes camera configuration. The server detects removal and stops the camera stream.

**Response:** Success or error message

---

## 🎮 Camera Control API

These endpoints control individual cameras using their configured path. Authentication via Bearer token if camera has `token` configured.

**Base Path:** `/{camera_path}/control`

### Recording Controls

#### Start Recording
**Endpoint:** `POST /{camera_path}/control/recording/start`

**Request Body (optional):**
```json
{
  "reason": "Motion detected"
}
```

**Response:**
```json
{
  "status": "success",
  "data": {
    "session_id": 123,
    "message": "Recording started",
    "camera_id": "cam1"
  }
}
```

#### Stop Recording
**Endpoint:** `POST /{camera_path}/control/recording/stop`

**Response:** Success message

#### Get Active Recording
**Endpoint:** `GET /{camera_path}/control/recording/active`

**Response:** Active recording info or message indicating none active

#### Get Recording Database Size
**Endpoint:** `GET /{camera_path}/control/recording/size`

**Response:**
```json
{
  "status": "success",
  "data": {
    "camera_id": "cam1",
    "size_bytes": 10485760,
    "size_mb": 10.0,
    "size_gb": 0.009765625
  }
}
```

### Recording Queries

#### List Recordings
**Endpoint:** `GET /{camera_path}/control/recordings`

**Query Parameters:**
- `from` (optional): ISO 8601 timestamp filter (recordings starting after this time)
- `to` (optional): ISO 8601 timestamp filter (recordings starting before this time)  
- `reason` (optional): Filter by recording reason using SQL wildcards (e.g., `Manual` or `%alarm%`)
- `sort_order` (optional): Sort order: `newest` (default) or `oldest`

**Response:** List of recording session objects with `keep_session` flag indicating protection status

**Examples:**
```bash
# Get all recordings
GET /cam1/control/recordings

# Get recordings from a specific time range
GET /cam1/control/recordings?from=2025-08-21T00:00:00Z&to=2025-08-21T23:59:59Z

# Get recordings with specific reasons
GET /cam1/control/recordings?reason=Manual
GET /cam1/control/recordings?reason=%alarm%

# Combined filters with sorting
GET /cam1/control/recordings?from=2025-08-21T00:00:00Z&reason=Manual&sort_order=oldest
```

#### Set Session Keep/Protection Flag
**Endpoint:** `PUT /{camera_path}/control/recordings/{session_id}/keep`

This endpoint allows you to mark a recording session as protected from automatic purging. Protected sessions will not be deleted by the cleanup process, ensuring important recordings are preserved.

**Query Parameters:**
- `keep` (optional): Set to `false` to remove protection. If omitted or any other value, protection is enabled.

**Response:**
```json
{
  "status": "success",
  "data": {
    "session_id": 123,
    "keep_session": true,
    "message": "Session 123 is now protected from purging"
  }
}
```

**Examples:**
```bash
# Protect a recording session (default behavior)
PUT /cam1/control/recordings/123/keep
Authorization: Bearer your-camera-token

# Remove protection from a session
PUT /cam1/control/recordings/123/keep?keep=false
Authorization: Bearer your-camera-token
```

#### Get Frame Metadata
**Endpoint:** `GET /{camera_path}/control/recordings/{session_id}/frames`

**Query Parameters:**
- `from` (optional): ISO 8601 timestamp
- `to` (optional): ISO 8601 timestamp

**Response:** List of frame metadata objects (timestamp, size)

#### Get Single Frame by Timestamp
**Endpoint:** `GET /{camera_path}/control/recordings/frames/{timestamp}`

**Path Parameters:**
- `timestamp`: ISO 8601 timestamp (URL-encoded, e.g., `2025-08-23T10:30:45.123Z`)

**Query Parameters:**
- `tolerance` (optional): Time tolerance for matching frames (default: exact match)
  - Format: `{number}{unit}` where unit is `s` (seconds), `m` (minutes), or `h` (hours)
  - Examples: `30s`, `5m`, `1h`
  - If exact timestamp not found, returns closest frame within tolerance

**Response:** 
- **Success (200)**: Raw JPEG binary data with headers:
  - `Content-Type: image/jpeg`
  - `X-Frame-Timestamp: {actual_frame_timestamp}`
- **Not Found (404)**: JSON error message
- **Bad Request (400)**: Invalid timestamp or tolerance format

**Examples:**
```bash
# Get exact frame at timestamp
GET /cam1/control/recordings/frames/2025-08-23T10:30:45.123Z

# Get closest frame within 30 seconds tolerance
GET /cam1/control/recordings/frames/2025-08-23T10:30:45.123Z?tolerance=30s

# Get closest frame within 5 minutes tolerance
GET /cam1/control/recordings/frames/2025-08-23T10:30:45.123Z?tolerance=5m

# With authentication
GET /cam1/control/recordings/frames/2025-08-23T10:30:45.123Z?tolerance=1h
Authorization: Bearer your-camera-token
```

#### List MP4 Segments
**Endpoint:** `GET {camera_path}/control/recordings/mp4/segments`

Advanced filtering for MP4 video segments.

**Query Parameters:**
- `from` (optional): ISO 8601 timestamp (segments ending after this time)
- `to` (optional): ISO 8601 timestamp (segments starting before this time)
- `reason` (optional): Filter by recording reason with SQL wildcards
- `limit` (optional): Max results (default: 1000)
- `sort_order` (optional): `newest` (default) or `oldest`

**Response:**
```json
{
  "status": "success",
  "data": {
    "segments": [
      {
        "id": "2_1724214554",
        "session_id": 2,
        "start_time": "2025-08-21T05:39:14.610108Z",
        "end_time": "2025-08-21T06:00:00.084373Z",
        "duration_seconds": 1245,
        "url": "/cam1/control/recordings/mp4/segments/2025-08-21T05-39-14Z.mp4",
        "size_bytes": 25653248,
        "recording_reason": "Manual recording started from dashboard",
        "camera_id": "cam1"
      }
    ],
    "count": 1,
    "camera_id": "cam1",
    "query": {
      "from": "2025-08-21T00:00:00.000Z",
      "to": "2025-08-21T23:59:59.999Z",
      "reason": null,
      "limit": 1000,
      "sort_order": "newest"
    }
  }
}
```

**Examples:**
```bash
# Get all segments
GET /cam1/control/recordings/mp4/segments

# Date range and reason filter
GET /cam1/control/recordings/mp4/segments?from=2025-08-21T00:00:00Z&to=2025-08-21T23:59:59Z&reason=Manual&limit=100

# Search for alarm segments
GET /cam1/control/recordings/mp4/segments?reason=%alarm%&sort_order=oldest
```

---

## 🎛️ PTZ Control API

Available only for cameras with PTZ capabilities enabled.

**Base Path:** `/{camera_path}/control/ptz`

### Move Camera
**Endpoint:** `POST /{camera_path}/control/ptz/move`

**Request Body:**
```json
{
  "pan": -1.0,        // -1.0 to 1.0
  "tilt": 0.5,        // -1.0 to 1.0  
  "zoom": 0.0,        // -1.0 to 1.0
  "timeout_secs": 5   // Movement duration
}
```

### Stop Movement
**Endpoint:** `POST /{camera_path}/control/ptz/stop`

### Go to Preset
**Endpoint:** `POST /{camera_path}/control/ptz/goto_preset`

**Request Body:**
```json
{
  "token": "preset-token"
}
```

### Set Preset
**Endpoint:** `POST /{camera_path}/control/ptz/set_preset`

**Request Body:**
```json
{
  "name": "Home",     // Optional
  "token": "home"     // Optional  
}
```

**Note:** Include `Authorization: Bearer <token>` header if camera has token configured.

---

## 📋 General API Information

### Authentication
- **Admin APIs**: Require `Authorization: Bearer <admin_token>` header
- **Camera APIs**: Require `Authorization: Bearer <camera_token>` header if camera has token configured
- **Video Streaming**: No authentication required (public endpoints)

### Response Formats
- **Success**: JSON with `status: "success"` and `data` object
- **Error**: JSON with `status: "error"` and `message` string
- **Video Content**: Binary streams with appropriate MIME types

### Timestamps
- All timestamps use ISO 8601 format: `2025-08-21T05:00:00Z`
- Query parameters accept both with and without milliseconds
- Responses include full precision timestamps

### CORS
- All endpoints include CORS headers for cross-origin requests
- Video streaming endpoints specifically allow `Access-Control-Allow-Origin: *`