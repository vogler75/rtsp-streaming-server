# REST API Documentation

This document describes all REST API endpoints available in the RTSP Streaming Server.

## Authentication

### Camera-Specific Endpoints
Camera endpoints require authentication via the `Authorization` header with a Bearer token:
```
Authorization: Bearer <camera_token>
```

### Admin Endpoints
Admin endpoints require authentication with the admin token configured in `config.toml`:
```
Authorization: Bearer <admin_token>
```
If no admin token is configured, admin endpoints are accessible without authentication.

## Response Format

All API responses follow this JSON structure:
```json
{
  "status": "success|error",
  "data": {}, // Present on success
  "error": "Error message", // Present on error
  "code": 400 // HTTP status code (present on error)
}
```

## Global Endpoints

### GET /api/status
Returns server status and camera information.

**Response:**
```json
{
  "status": "success",
  "data": {
    "server": {
      "uptime": "2h 30m 45s",
      "version": "1.0.0"
    },
    "cameras": {
      "cam1": {
        "status": "connected|disconnected|error",
        "last_frame": "2023-12-01T10:30:00Z",
        "fps": 15.2,
        "error": "Optional error message"
      }
    }
  }
}
```

### GET /api/cameras
Returns list of all configured cameras with their configurations.

**Response:**
```json
{
  "status": "success",
  "data": {
    "cameras": {
      "cam1": {
        "name": "Camera 1",
        "path": "/cam1",
        "url": "rtsp://camera-ip:554/stream",
        "enabled": true,
        "transport": "tcp",
        "reconnect_interval": 5000,
        "status": "connected|disconnected|error"
      }
    }
  }
}
```

## Camera Recording Control Endpoints

These endpoints are available for each camera at `/<camera_path>/control/api/*`:

### POST /<camera_path>/control/api/start
Start recording for the camera.

**Headers:** `Authorization: Bearer <camera_token>`

**Request Body:**
```json
{
  "reason": "Optional recording reason"
}
```

**Response (Success):**
```json
{
  "status": "success",
  "data": {
    "message": "Recording started",
    "session_id": 123
  }
}
```

**Response (Already Recording):**
```json
{
  "status": "error",
  "error": "Recording already in progress for this camera",
  "code": 409
}
```

### POST /<camera_path>/control/api/stop
Stop recording for the camera.

**Headers:** `Authorization: Bearer <camera_token>`

**Response:**
```json
{
  "status": "success",
  "data": {
    "message": "Recording stopped"
  }
}
```

### GET /<camera_path>/control/api/recordings
List recordings for the camera.

**Headers:** `Authorization: Bearer <camera_token>`

**Query Parameters:**
- `from` (optional): Start time filter (ISO 8601 format)
- `to` (optional): End time filter (ISO 8601 format)

**Response:**
```json
{
  "status": "success",
  "data": {
    "recordings": [
      {
        "id": 123,
        "camera_id": "cam1",
        "start_time": "2023-12-01T10:00:00Z",
        "end_time": "2023-12-01T10:30:00Z",
        "reason": "Motion detected"
      }
    ]
  }
}
```

### GET /<camera_path>/control/api/recordings/<session_id>/frames
Get recorded frames for a specific recording session.

**Headers:** `Authorization: Bearer <camera_token>`

**Query Parameters:**
- `from` (optional): Start time filter (ISO 8601 format)
- `to` (optional): End time filter (ISO 8601 format)

**Response:**
```json
{
  "status": "success",
  "data": {
    "frames": [
      {
        "timestamp": "2023-12-01T10:00:01Z",
        "frame_data": "base64_encoded_frame_data"
      }
    ]
  }
}
```

### GET /<camera_path>/control/api/active
Get active recording information for the camera.

**Headers:** `Authorization: Bearer <camera_token>`

**Response (Active Recording):**
```json
{
  "status": "success",
  "data": {
    "recording": {
      "id": 123,
      "camera_id": "cam1",
      "start_time": "2023-12-01T10:00:00Z",
      "reason": "Manual start"
    }
  }
}
```

**Response (No Active Recording):**
```json
{
  "status": "success",
  "data": {
    "recording": null
  }
}
```

### GET /<camera_path>/control/api/size
Get total storage size used by recordings for the camera.

**Headers:** `Authorization: Bearer <camera_token>`

**Response:**
```json
{
  "status": "success",
  "data": {
    "size_bytes": 1048576,
    "size_mb": 1.0,
    "size_gb": 0.001
  }
}
```

## Admin Camera Management Endpoints

### POST /api/admin/cameras
Create a new camera configuration.

**Headers:** `Authorization: Bearer <admin_token>`

**Request Body:**
```json
{
  "camera_id": "new_camera",
  "config": {
    "enabled": true,
    "path": "/new_camera",
    "url": "rtsp://camera-ip:554/stream",
    "transport": "tcp",
    "reconnect_interval": 5000,
    "token": "camera_access_token",
    "ffmpeg": {
      "output_format": "mjpeg",
      "video_codec": "mjpeg",
      "quality": 80,
      "output_framerate": 15
    }
  }
}
```

**Response:**
```json
{
  "status": "success",
  "data": {
    "message": "Camera created successfully",
    "camera_id": "new_camera"
  }
}
```

### GET /api/admin/cameras/:id
Get configuration for a specific camera.

**Headers:** `Authorization: Bearer <admin_token>`

**Response:**
```json
{
  "status": "success",
  "data": {
    "enabled": true,
    "path": "/cam1",
    "url": "rtsp://camera-ip:554/stream",
    "transport": "tcp",
    "reconnect_interval": 5000,
    "token": "camera_token"
  }
}
```

### PUT /api/admin/cameras/:id
Update configuration for a specific camera.

**Headers:** `Authorization: Bearer <admin_token>`

**Request Body:** Same as camera config in POST request

**Response:**
```json
{
  "status": "success",
  "data": {
    "message": "Camera updated successfully"
  }
}
```

### DELETE /api/admin/cameras/:id
Delete a camera configuration.

**Headers:** `Authorization: Bearer <admin_token>`

**Response:**
```json
{
  "status": "success",
  "data": {
    "message": "Camera deleted successfully"
  }
}
```

## Error Responses

### Authentication Errors
```json
{
  "status": "error",
  "error": "Invalid or missing Authorization header",
  "code": 401
}
```

### Not Found Errors
```json
{
  "status": "error",
  "error": "Camera not found",
  "code": 404
}
```

### Conflict Errors
```json
{
  "status": "error",
  "error": "Camera already exists",
  "code": 409
}
```

### Validation Errors
```json
{
  "status": "error",
  "error": "Path and URL are required",
  "code": 400
}
```

## Camera Configuration Schema

### CameraConfig Object
```json
{
  "enabled": true, // Optional, defaults to true
  "path": "/cam1", // Required - URL path for camera endpoints
  "url": "rtsp://camera-ip:554/stream", // Required - RTSP URL
  "transport": "tcp", // Required - "tcp" or "udp"
  "reconnect_interval": 5000, // Required - Reconnection interval in ms
  "chunk_read_size": 8192, // Optional - RTSP chunk size in bytes
  "token": "access_token", // Optional - Authentication token
  "max_recording_age": "7d", // Optional - Recording retention (e.g., "10m", "5h", "7d")
  "ffmpeg": { // Optional - FFmpeg transcoding settings
    "command": "Custom FFmpeg command", // Optional - Full command override
    "use_wallclock_as_timestamps": true, // Optional
    "output_format": "mjpeg", // Optional - Output format
    "video_codec": "mjpeg", // Optional - Video codec
    "video_bitrate": "1M", // Optional - Video bitrate
    "quality": 80, // Optional - JPEG quality (1-100)
    "output_framerate": 15, // Optional - Output FPS
    "scale": "640:480", // Optional - Video scaling
    "rtbufsize": 102400, // Optional - RTSP buffer size
    "extra_input_args": ["--arg1"], // Optional - Additional input args
    "extra_output_args": ["--arg2"], // Optional - Additional output args
    "log_stderr": "console" // Optional - "file", "console", "both"
  },
  "mqtt": { // Optional - MQTT publishing settings
    "enabled": true,
    "status_topic": "cameras/cam1/status",
    "image_topic": "cameras/cam1/image",
    "publish_interval": 30000
  }
}
```

## WebSocket Endpoints

While not REST endpoints, the following WebSocket endpoints are available:

- `/<camera_path>/stream` - Video streaming interface (HTML page)
- `/<camera_path>/control` - Recording control interface (HTML page) 
- `/<camera_path>/live` - Raw WebSocket video stream (requires `token` query parameter)

## Static Endpoints

- `/dashboard` - Multi-camera overview page
- `/admin` - Camera management interface
- `/<camera_path>` - Individual camera test page
- `/<camera_path>/test` - Alternative camera test page
- `/static/*` - Static file serving