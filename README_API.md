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
