# REST API Endpoints

This document provides a hierarchical overview of the available REST API endpoints.

## API Endpoint Hierarchy

*   **`/`**
    *   `GET /dashboard`: Displays the main dashboard.
*   **`/api`**
    *   `GET /status`: Retrieves the current status of the server.
    *   `GET /cameras`: Retrieves a list of all available cameras.
    *   **`/recordings`**
        *   `GET /:camera_id/:filename`: Streams a specific MP4 recording.
    *   **`/cameras`**
        *   **`/:camera_id`**
            *   **`/recordings`**
                *   `POST /start`: Starts recording for the specified camera.
                *   `POST /stop`: Stops recording for the specified camera.
                *   `GET /`: Lists all recordings for the specified camera.
                *   `GET /active`: Checks if a recording is currently active for the specified camera.
                *   **`/:filename`**
                    *   `GET /frames`: Retrieves the frames of a specific recording.
                    *   `GET /size`: Retrieves the size of a specific recording.
    *   **`/admin`**
        *   **`/cameras`**
            *   `POST /`: Creates a new camera configuration.
            *   **`/:id`**
                *   `GET /`: Retrieves the configuration for a specific camera.
                *   `PUT /`: Updates the configuration for a specific camera.
                *   `DELETE /`: Deletes a specific camera.
        *   **`/config`**
            *   `GET /`: Retrieves the server's main configuration.
            *   `PUT /`: Updates the server's main configuration.
*   **`/stream`**
    *   `GET /:camera_id`: Serves the video stream page for a specific camera.
*   **`/control`**
    *   `GET /:camera_id`: Serves the control page for a specific camera.
*   **`/live`**
    *   `GET /:camera_id`: Serves the live HLS stream for a specific camera.
*   **`/test`**
    *   `GET /`: Serves a test page.
    *   `GET /:camera_id`: Serves a test page for a specific camera.


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
