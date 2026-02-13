# VideoPlayer Custom Web Control (CWC)

A Siemens WinCC Unified Custom Web Control for video streaming and playback with RTSP streaming server integration.

## Overview

This Custom Web Control provides a video player interface that can connect to RTSP streaming servers via WebSocket connections. It supports live streaming, HLS streaming, and recorded video playback with advanced control features including PTZ camera control.

## Properties

The VideoPlayer CWC exposes the following properties that can be configured and bound in WinCC Unified. The property names listed here match the manifest and are the names used in WinCC Unified engineering.

### Connection Properties

#### `camera_stream_url` (string)
- **Description**: WebSocket connection URL for the video stream
- **Default**: `""` (empty string)
- **Example**: `"ws://localhost:8080/cam1"` or `"wss://server.example.com/camera1"`
- **Usage**: Set this to the base camera path. The control will automatically append `/control` if `use_control_stream` is set to true.

#### `camera_auth_token` (string)
- **Description**: Authentication token for WebSocket connection (Bearer token)
- **Default**: `""` (empty string)
- **Example**: `"your-camera-access-token"`
- **Usage**: Required if your camera endpoint uses token-based authentication

#### `enable_connection` (boolean)
- **Description**: Enable/disable video connection
- **Default**: `false`
- **Usage**: Set to `true` to initiate connection, `false` to disconnect

#### `status_connected` (boolean, read-only)
- **Description**: Indicates the current connection state
- **Default**: `false`
- **Usage**: Monitor this property to check if the video stream is connected

### Display Properties

#### `show_version` (boolean)
- **Description**: Show/hide build version number in the top-right corner
- **Default**: `false`
- **Usage**: Set to `true` to display version information for debugging

#### `enable_debug` (boolean)
- **Description**: Enable debug logging for troubleshooting
- **Default**: `false`
- **Usage**: Set to `true` to enable detailed console logging

### Performance Monitoring

#### `status_fps` (number, read-only)
- **Description**: Current frames per second of the video stream
- **Default**: `0`
- **Usage**: Monitor video stream performance

#### `status_bitrate_kbps` (number, read-only)
- **Description**: Current bitrate in kilobits per second
- **Default**: `0`
- **Usage**: Monitor bandwidth usage

### Streaming Mode Properties

#### `use_control_stream` (boolean)
- **Description**: Enable control mode instead of normal video streaming
- **Default**: `false`
- **Usage**: Set to `true` to enable advanced playback controls for recorded video. When true, the control appends `/control` to the URL.

#### `use_hls_streaming` (boolean)
- **Description**: Enable HLS player mode instead of WebSocket streaming
- **Default**: `false`
- **Usage**: Set to `true` to use HLS (HTTP Live Streaming) for video playback instead of WebSocket-based MJPEG streaming. Requires the server to have HLS storage enabled.

### Playback Control Properties

*These properties are only effective when `use_control_stream` is set to `true`*

#### `playback_start_time` (string)
- **Description**: Start timestamp for playback as ISO string
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T10:00:00.000Z"`
- **Usage**: Set the beginning timestamp for recorded video playback

#### `playback_end_time` (string)
- **Description**: End timestamp for playback as ISO string (optional)
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T12:00:00.000Z"`
- **Usage**: Set the ending timestamp for recorded video playback. If empty, plays until end

#### `enable_playback` (boolean)
- **Description**: Start/stop playback with the specified time range
- **Default**: `false`
- **Usage**: Set to `true` to start playback, `false` to stop

#### `playback_speed` (number)
- **Description**: Playback speed multiplier
- **Default**: `1.0`
- **Range**: `0.1` to `10.0`
- **Examples**:
  - `1.0` = normal speed
  - `2.0` = 2x speed (fast forward)
  - `0.5` = half speed (slow motion)
- **Usage**: Control playback speed for recorded video

#### `enable_livestream` (boolean)
- **Description**: Enable/disable live stream mode
- **Default**: `false`
- **Usage**: Set to `true` to switch to live streaming mode

#### `seek_to_time` (string)
- **Description**: Jump to specific timestamp in ISO format
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T10:30:00.000Z"`
- **Usage**: Set to jump to a specific point in recorded video

#### `status_timestamp` (string, read-only)
- **Description**: Current video frame timestamp in ISO format
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T10:30:15.123Z"`
- **Usage**: Monitor the current playback position

### Recording Control Properties

*These properties are only effective when `use_control_stream` is set to `true`*

#### `recording_reason` (string)
- **Description**: Reason for starting a recording session
- **Default**: `""` (empty string)
- **Example**: `"Alarm triggered"` or `"Manual recording"`
- **Usage**: Set a descriptive reason before starting recording

#### `enable_recording` (boolean)
- **Description**: Start or stop recording
- **Default**: `false`
- **Usage**: Set to `true` to start recording, `false` to stop recording

### PTZ Control Properties

*These properties are available for cameras that support Pan-Tilt-Zoom control*

#### `ptz_move` (string)
- **Description**: PTZ move command as JSON string
- **Default**: `""` (empty string)
- **Example**: `'{"pan": 1, "tilt": 0, "zoom": 0}'` or `'{"pan": -0.5, "tilt": 0.3, "zoom": 0.2}'`
- **Usage**: Set JSON command to move camera. Values typically range from -1.0 to 1.0

#### `ptz_stop` (boolean)
- **Description**: Stop all PTZ movements
- **Default**: `false`
- **Usage**: Set to `true` to stop all camera movements. Automatically resets to `false` after command is sent

#### `ptz_goto_preset` (string)
- **Description**: Go to PTZ preset as JSON string
- **Default**: `""` (empty string)
- **Example**: `'{"preset": 1}'` or `'{"preset": "entrance"}'`
- **Usage**: Set JSON command to move camera to a saved preset position

#### `ptz_set_preset` (string)
- **Description**: Set/save current position as PTZ preset
- **Default**: `""` (empty string)
- **Example**: `'{"preset": 1, "name": "Entrance"}'` or `'{"preset": "parking", "name": "Parking Area"}'`
- **Usage**: Set JSON command to save current camera position as a preset

## Usage Examples

### Basic Live Streaming

```javascript
// Configure basic live streaming
control.properties.camera_stream_url = "ws://localhost:8080/cam1";
control.properties.camera_auth_token = "your-access-token";
control.properties.enable_connection = true;
```

### HLS Streaming

```javascript
// Use HLS streaming mode
control.properties.camera_stream_url = "ws://localhost:8080/cam1";
control.properties.use_hls_streaming = true;
control.properties.enable_connection = true;
```

### Recorded Video Playback

```javascript
// Enable control mode for recorded video
control.properties.camera_stream_url = "ws://localhost:8080/cam1";
control.properties.use_control_stream = true;
control.properties.enable_connection = true;

// Set playback time range
control.properties.playback_start_time = "2025-08-15T10:00:00.000Z";
control.properties.playback_end_time = "2025-08-15T12:00:00.000Z";
control.properties.playback_speed = 1.0;
control.properties.enable_playback = true;
```

### Recording Control

```javascript
// Start a recording with reason
control.properties.recording_reason = "Security incident";
control.properties.enable_recording = true;

// Stop recording
control.properties.enable_recording = false;
```

### PTZ Camera Control

```javascript
// Move camera (pan right, tilt up slightly)
control.properties.ptz_move = '{"pan": 0.5, "tilt": 0.2, "zoom": 0}';

// Stop all camera movements
control.properties.ptz_stop = true;

// Go to preset position 1
control.properties.ptz_goto_preset = '{"preset": 1}';

// Save current position as preset 2 with name
control.properties.ptz_set_preset = '{"preset": 2, "name": "Main Entrance"}';

// Zoom in
control.properties.ptz_move = '{"pan": 0, "tilt": 0, "zoom": 0.3}';

// Pan left at half speed
control.properties.ptz_move = '{"pan": -0.5, "tilt": 0, "zoom": 0}';
```

### Performance Monitoring

```javascript
// Monitor connection and performance
if (control.properties.status_connected) {
    console.log(`FPS: ${control.properties.status_fps}`);
    console.log(`Bitrate: ${control.properties.status_bitrate_kbps} kbps`);
    console.log(`Current time: ${control.properties.status_timestamp}`);
}
```

## URL Structure

The control automatically constructs the full endpoint URL based on the base URL and streaming mode:

- **Base URL**: Set via `camera_stream_url` (e.g., `ws://localhost:8080/cam1`)
- **Stream Mode** (`use_control_stream=false`): Uses the base URL directly
- **Control Mode** (`use_control_stream=true`): Appends `/control` to base URL

### Additional Endpoints

The base URL approach supports multiple endpoints for different functionality:
- **PTZ Control**: `/camera-path/control/ptz/{command}` (move, stop, goto-preset, set-preset)

Example usage:
```javascript
// For streaming mode
control.properties.camera_stream_url = "ws://localhost:8080/cam1";
control.properties.use_control_stream = false;  // Uses ws://localhost:8080/cam1

// For control mode
control.properties.camera_stream_url = "ws://localhost:8080/cam1";
control.properties.use_control_stream = true;   // Uses ws://localhost:8080/cam1/control
```

## Authentication

If your RTSP streaming server requires authentication:

1. Set the `camera_auth_token` property with your access token
2. The control will automatically include it in WebSocket connection headers
3. The token can also be passed as a query parameter

## Connection Reliability

The VideoPlayer CWC includes robust auto-reconnection features:

### Auto-Reconnection
- **Unlimited Attempts**: Reconnects endlessly until successful or manually stopped
- **Fixed Interval**: Simple 1-second delay between all attempts
- **Visual Feedback**: Shows reconnection countdown and attempt counter
- **Authentication Handling**: Won't retry on authentication failures (codes 1002, 1003)

### Connection Status Messages
- **Connected**: Status hidden in normal mode, smart status in debug mode (green):
  - Stream mode: "Live Stream"
  - Control mode (not live): "Connected"
  - Control mode (live active): "Live Stream"
- **Disconnected**: "Disconnected" (red) or "Stopped" (gray) when connection disabled
- **Reconnecting**: "Reconnecting in 1s... (N)" (orange) with attempt counter

### Smart Connection Management
- URL changes automatically close and reopen connections
- Control/stream mode switching triggers reconnection with proper endpoint
- Livestream state changes update status display in real-time

## Troubleshooting

### Enable Debug Mode
```javascript
control.properties.enable_debug = true;
control.properties.show_version = true;
```

### Common Issues

1. **Connection Fails**: Check URL format and server availability
2. **Authentication Errors**: Verify token is correct and not expired (won't auto-retry)
3. **No Video**: Ensure camera is configured and streaming
4. **Performance Issues**: Monitor `status_fps` and `status_bitrate_kbps` properties
5. **Frequent Reconnections**: Check network stability and server load

### Connection States

- **Immediate Connection**: When `enable_connection=true` and URL is valid
- **Auto-Reconnection**: Triggered by network errors or abnormal closures
- **No Reconnection**: On authentication failures or normal closures
- **Manual Reconnection**: Toggle `enable_connection` property off/on to reset attempt counter

### SSL/TLS Issues

The control automatically handles certificate issues by falling back from WSS to WS connections when necessary.

## Build Information

- **Version**: 1.0.54
- **Type**: Custom Web Control for Siemens WinCC Unified
- **GUID**: `551BF148-3F0D-4293-99C2-C9C3A1A6A073`

## Files Structure

```
cwc/
├── src/
│   ├── manifest.json          # CWC manifest and property definitions
│   ├── control/
│   │   ├── index.html         # Main HTML interface
│   │   ├── code.js            # JavaScript control logic
│   │   └── styles.css         # Styling
│   └── assets/
│       └── logo.ico           # Control icon
├── build.sh                   # Build script
└── README.md                  # This documentation
```

## Integration with RTSP Streaming Server

This CWC is designed to work with the RTSP streaming server in this repository. Make sure:

1. The RTSP streaming server is running
2. Cameras are properly configured
3. WebSocket endpoints are accessible
4. Authentication tokens match camera configurations

For more information about the RTSP streaming server, see the main project documentation.
