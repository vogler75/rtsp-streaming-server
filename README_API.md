# REST API Endpoints

This document provides a hierarchical overview of the available REST API endpoints.

## API Endpoint Hierarchy

```text
/
  GET  /dashboard                         # Dashboard page

/api
  GET  /status                            # Server status
  GET  /cameras                           # List cameras
  /recordings
    GET  /:camera_id/:filename            # Stream MP4 recording
  /admin
    /cameras
      POST /                              # Create camera
      GET  /:id                           # Get camera config
      PUT  /:id                           # Update camera config
      DELETE /:id                         # Delete camera
    /config
      GET  /                              # Get server config
      PUT  /                              # Update server config

# Per-camera routes (use the configured camera path, e.g., /cam1)
<camera_path>
  GET  /                                  # Camera test page
  GET  /stream                            # Stream page (WebSocket used for frames)
  GET  /control                           # Control page (WebSocket used for control)
  GET  /live                              # Live stream over WebSocket
  GET  /test                              # Alternate test page

<camera_path>/control
  # Recording API
  POST /recording/start                   # Start recording
  POST /recording/stop                    # Stop recording
  GET  /recordings                        # List recordings
  GET  /recording/active                  # Active recording status
  GET  /recording/size                    # Recording DB size
  GET  /recordings/:session_id/frames     # Frames metadata for a recording

<camera_path>/recordings
  GET  /mp4/segments                      # List MP4 video segments

  # PTZ API (if enabled for the camera)
  POST /ptz/move                          # Continuous pan/tilt/zoom
  POST /ptz/stop                          # Stop movement
  POST /ptz/goto_preset                   # Move to preset by token
  POST /ptz/set_preset                    # Create/update a preset
```


## Camera Management API (`/api/admin/cameras`)

These endpoints are used for managing camera configurations and require an admin token set in the `Authorization` header (e.g., `Authorization: Bearer <your_admin_token>`).

### `POST /api/admin/cameras`

Creates a new camera configuration. The server will automatically detect the new configuration file and start the camera stream.

- **Method**: `POST`
- **Path**: `/api/admin/cameras`
- **Request Body**:
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
- **Response**: A success or error message.

### `GET /api/admin/cameras/:id`

Retrieves the current configuration for a specific camera.

- **Method**: `GET`
- **Path**: `/api/admin/cameras/:id` (e.g., `/api/admin/cameras/cam1`)
- **Response**: The camera's configuration object.

### `PUT /api/admin/cameras/:id`

Updates the configuration for a specific camera. The server will detect the change and restart the camera stream.

- **Method**: `PUT`
- **Path**: `/api/admin/cameras/:id`
- **Request Body**: A `CameraConfig` JSON object.
- **Response**: A success or error message.

### `DELETE /api/admin/cameras/:id`

Deletes a camera's configuration file. The server will detect the removal and stop the corresponding camera stream.

- **Method**: `DELETE`
- **Path**: `/api/admin/cameras/:id`
- **Response**: A success or error message.

## Camera Control API

These endpoints are available for each camera, identified by its configured `path`. If a `token` is configured for the camera, it must be provided in the `Authorization` header as a Bearer token.

### `POST /<camera_path>/control/recording/start`

Starts a new recording session for the camera.

- **Method**: `POST`
- **Path**: `/<camera_path>/control/recording/start` (e.g., `/cam1/control/recording/start`)
- **Request Body** (optional):
  ```json
  {
    "reason": "Motion detected"
  }
  ```
- **Response**:
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

### `POST /<camera_path>/control/recording/stop`

Stops the currently active recording session for the camera.

- **Method**: `POST`
- **Path**: `/<camera_path>/control/recording/stop`
- **Response**: A success message indicating the recording has been stopped.

### `GET /<camera_path>/control/recordings`

Lists all recorded sessions for the camera.

- **Method**: `GET`
- **Path**: `/<camera_path>/control/recordings`
- **Query Parameters**:
  - `from`: (Optional) ISO 8601 timestamp to filter recordings that started after this time.
  - `to`: (Optional) ISO 8601 timestamp to filter recordings that started before this time.
- **Response**: A list of recording session objects.

### `GET /<camera_path>/control/recordings/:session_id/frames`

Retrieves metadata for frames within a specific recording session. Note that this does not return the actual frame data.

- **Method**: `GET`
- **Path**: `/<camera_path>/control/recordings/:session_id/frames`
- **Query Parameters**:
  - `from`: (Optional) ISO 8601 timestamp.
  - `to`: (Optional) ISO 8601 timestamp.
- **Response**: A list of frame metadata objects (timestamp, size).

### `GET /<camera_path>/control/recording/active`

Gets the status of the currently active recording for the camera.

- **Method**: `GET`
- **Path**: `/<camera_path>/control/recording/active`
- **Response**: Information about the active recording session, or a message indicating no active recording.

### `GET /<camera_path>/control/recording/size`

Gets the total size of the recording database for the camera.

- **Method**: `GET`
- **Path**: `/<camera_path>/control/recording/size`
- **Response**:
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

### `GET /<camera_path>/recordings/mp4/segments`

Lists MP4 video segments for the camera with advanced filtering options.

- **Method**: `GET`
- **Path**: `/<camera_path>/recordings/mp4/segments` (e.g., `/cam1/recordings/mp4/segments`)
- **Query Parameters**:
  - `from`: (Optional) ISO 8601 timestamp to filter segments that end after this time.
  - `to`: (Optional) ISO 8601 timestamp to filter segments that start before this time.
  - `reason`: (Optional) Filter by recording reason using SQL wildcards (e.g., `Manual` or `%alarm%`).
  - `limit`: (Optional) Maximum number of results to return (default: 1000).
  - `sort_order`: (Optional) Sort order: `newest` (default) or `oldest`.
- **Response**:
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
          "file_path": "recordings/cam1/2025/08/21/2025-08-21T05-39-14Z.mp4",
          "size_bytes": 25653248,
          "recording_reason": "Manual recording started from dashboard",
          "camera_id": "cam1",
          "url": "/api/recordings/cam1/2025-08-21T05-39-14Z.mp4"
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
- **Example Usage**:
  ```bash
  # Get all segments for cam1
  GET /cam1/recordings/mp4/segments
  
  # Get segments with date range and reason filter
  GET /cam1/recordings/mp4/segments?from=2025-08-21T00:00:00Z&to=2025-08-21T23:59:59Z&reason=Manual&limit=100
  
  # Search for segments containing "alarm" in the reason
  GET /cam1/recordings/mp4/segments?reason=%alarm%&sort_order=oldest
  ```

### PTZ Endpoints (if enabled)

All PTZ endpoints live under the camera control path: `/<camera_path>/control/ptz/*`.

- `POST /<camera_path>/control/ptz/move`
  - Body: `{ "pan": -1.0..1.0, "tilt": -1.0..1.0, "zoom": -1.0..1.0, "timeout_secs": 0.. }`
- `POST /<camera_path>/control/ptz/stop`
- `POST /<camera_path>/control/ptz/goto_preset`
  - Body: `{ "token": "preset-token" }`
- `POST /<camera_path>/control/ptz/set_preset`
  - Body: `{ "name": "Home", "token": "home" }` (either field optional)

Note: If a `token` is configured for the camera, include `Authorization: Bearer <token>` when calling these endpoints.
