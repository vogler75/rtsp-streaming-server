# REST API Documentation

This document provides a comprehensive overview of all available REST API endpoints for the RTSP streaming server.

## 🎥 Quick Reference - Video Streaming

| Endpoint | Purpose | Format | Parameters |
|----------|---------|---------|------------|
| `/api/recordings/{camera_id}/mp4/segments/{filename}` | Single MP4 recording | MP4 | - |
| `/api/recordings/{camera_id}/hls/timerange` | HLS playlist for time range | M3U8 | `t1`, `t2`, `segment_duration` |

**Example:**
```bash
# HLS playlist for a time range
GET /api/recordings/cam1/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z
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
    ├── recordings/
    │   └── {camera_id}/
    │       ├── mp4/
    │       │   └── segments/
    │       │       └── {filename}            # Stream single MP4
    │       └── hls/
    │           ├── timerange                 # Generate HLS playlist
    │           └── segments/
    │               └── {playlist_id}/
    │                   └── {segment_name}    # Serve HLS segments
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
    │   └── mp4/
    │       └── segments                      # List MP4 segments
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

**Endpoint:** `GET /api/recordings/{camera_id}/mp4/segments/{filename}`

Stream an individual MP4 recording file for playback.

- **Authentication**: None required (public endpoint)
- **Headers**:
  - `Range` (optional): Byte-range requests for seeking (e.g., `bytes=0-1024`)
- **Response**: 
  - `200 OK`: Full video file
  - `206 Partial Content`: When Range header provided
  - `404 Not Found`: Recording not found
  - Headers: `Content-Type: video/mp4`, `Accept-Ranges: bytes`, `Cache-Control: public, max-age=3600`

**Examples:**
```bash
# Stream full MP4 file
GET /api/recordings/cam1/mp4/segments/2025-08-21T05-39-14Z.mp4

# Stream with byte range for seeking
GET /api/recordings/cam1/mp4/segments/2025-08-21T05-39-14Z.mp4
Range: bytes=1024-2048
```

### HLS Time Range Playlist

**Endpoint:** `GET /api/recordings/{camera_id}/hls/timerange`

Generate an HLS (HTTP Live Streaming) playlist for recordings within a time range.

- **Authentication**: None required (public endpoint)
- **Query Parameters**:
  - `t1` (required): Start time in ISO 8601 format
  - `t2` (required): End time in ISO 8601 format  
  - `segment_duration` (optional): Target segment duration in seconds (default: 10)
- **Response**: 
  - `200 OK`: M3U8 playlist content
  - `404 Not Found`: No recordings in time range
  - Headers: `Content-Type: application/vnd.apple.mpegurl`, `Access-Control-Allow-Origin: *`

**Features:**
- Creates MPEG-TS segments from MP4 recordings
- Compatible with HLS.js, Video.js, and native iOS/macOS players
- Supports adaptive bitrate streaming workflows
- Works with both database and filesystem storage

**Examples:**
```bash
# Basic HLS playlist
GET /api/recordings/cam1/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z

# Custom segment duration
GET /api/recordings/cam1/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z&segment_duration=5
```

**HTML5 Usage:**
```html
<script src="https://cdn.jsdelivr.net/npm/hls.js@latest"></script>
<video id="video" controls></video>
<script>
  const video = document.getElementById('video');
  const hls = new Hls();
  hls.loadSource('/api/recordings/cam1/hls/timerange?t1=2025-08-21T05:00:00Z&t2=2025-08-21T05:30:00Z');
  hls.attachMedia(video);
</script>
```

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
- `from` (optional): ISO 8601 timestamp filter
- `to` (optional): ISO 8601 timestamp filter

**Response:** List of recording session objects

#### Get Frame Metadata
**Endpoint:** `GET /{camera_path}/control/recordings/{session_id}/frames`

**Query Parameters:**
- `from` (optional): ISO 8601 timestamp
- `to` (optional): ISO 8601 timestamp

**Response:** List of frame metadata objects (timestamp, size)

#### List MP4 Segments
**Endpoint:** `GET /{camera_path}/recordings/mp4/segments`

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
        "url": "/api/recordings/cam1/mp4/segments/2025-08-21T05-39-14Z.mp4",
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
GET /cam1/recordings/mp4/segments

# Date range and reason filter
GET /cam1/recordings/mp4/segments?from=2025-08-21T00:00:00Z&to=2025-08-21T23:59:59Z&reason=Manual&limit=100

# Search for alarm segments
GET /cam1/recordings/mp4/segments?reason=%alarm%&sort_order=oldest
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