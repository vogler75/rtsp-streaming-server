# VideoPlayer Custom Web Control (CWC)

A Siemens WinCC Unified Custom Web Control for video streaming and playback with RTSP streaming server integration.

## Overview

This Custom Web Control provides a video player interface that can connect to RTSP streaming servers via WebSocket connections. It supports both live streaming and recorded video playback with advanced control features.

## Properties

The VideoPlayer CWC exposes the following properties that can be configured and bound in WinCC Unified:

### Connection Properties

#### `URL` (string)
- **Description**: Base WebSocket URL for the camera (without endpoint suffix)
- **Default**: `""` (empty string)
- **Example**: `"ws://localhost:8080/cam1"` or `"wss://server.example.com/camera1"`
- **Usage**: Set this to the base camera path. The control will automatically append `/stream` or `/control` based on the `control` property

#### `token` (string)
- **Description**: Authentication token for WebSocket connection (Bearer token)
- **Default**: `""` (empty string)
- **Example**: `"your-camera-access-token"`
- **Usage**: Required if your camera endpoint uses token-based authentication

#### `connect` (boolean)
- **Description**: Enable/disable video connection
- **Default**: `false`
- **Usage**: Set to `true` to initiate connection, `false` to disconnect

#### `connected` (boolean, read-only)
- **Description**: Indicates the current connection state
- **Default**: `false`
- **Usage**: Monitor this property to check if the video stream is connected

### Display Properties

#### `version` (boolean)
- **Description**: Show/hide build version number in the top-right corner
- **Default**: `false`
- **Usage**: Set to `true` to display version information for debugging

#### `debug` (boolean)
- **Description**: Enable debug logging for troubleshooting
- **Default**: `false`
- **Usage**: Set to `true` to enable detailed console logging

### Performance Monitoring

#### `fps` (number, read-only)
- **Description**: Current frames per second of the video stream
- **Default**: `0`
- **Usage**: Monitor video stream performance

#### `kbs` (number, read-only)
- **Description**: Current kilobytes per second of the video stream
- **Default**: `0`
- **Usage**: Monitor bandwidth usage

### Control Mode Properties

#### `control` (boolean)
- **Description**: Enable control mode instead of normal video streaming
- **Default**: `false`
- **Usage**: Set to `true` to enable advanced playback controls for recorded video. When true, the control appends `/control` to the URL; when false, it appends `/stream`

### Playback Control Properties

*These properties are only available when `control` is set to `true`*

#### `play_from` (string)
- **Description**: Start timestamp for playback as ISO string
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T10:00:00.000Z"`
- **Usage**: Set the beginning timestamp for recorded video playback

#### `play_to` (string)
- **Description**: End timestamp for playback as ISO string (optional)
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T12:00:00.000Z"`
- **Usage**: Set the ending timestamp for recorded video playback. If empty, plays until end

#### `play` (boolean)
- **Description**: Start/stop playback with the specified time range
- **Default**: `false`
- **Usage**: Set to `true` to start playback, `false` to stop

#### `speed` (number)
- **Description**: Playback speed multiplier
- **Default**: `1.0`
- **Range**: `0.1` to `10.0`
- **Examples**: 
  - `1.0` = normal speed
  - `2.0` = 2x speed (fast forward)
  - `0.5` = half speed (slow motion)
- **Usage**: Control playback speed for recorded video

#### `live` (boolean)
- **Description**: Enable/disable live stream mode
- **Default**: `false`
- **Usage**: Set to `true` to switch to live streaming mode

#### `goto` (string)
- **Description**: Jump to specific timestamp in ISO format
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T10:30:00.000Z"`
- **Usage**: Set to jump to a specific point in recorded video

#### `timestamp` (string, read-only)
- **Description**: Current video frame timestamp in ISO format
- **Default**: `""` (empty string)
- **Example**: `"2025-08-15T10:30:15.123Z"`
- **Usage**: Monitor the current playback position

### Recording Control Properties

*These properties are only available when `control` is set to `true`*

#### `recording_reason` (string)
- **Description**: Reason for starting a recording session
- **Default**: `""` (empty string)
- **Example**: `"Alarm triggered"` or `"Manual recording"`
- **Usage**: Set a descriptive reason before starting recording

#### `recording_active` (boolean)
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
control.properties.URL = "ws://localhost:8080/cam1/live";
control.properties.token = "your-access-token";
control.properties.connect = true;
```

### Recorded Video Playback

```javascript
// Enable control mode for recorded video
control.properties.URL = "ws://localhost:8080/cam1/control";
control.properties.control = true;
control.properties.connect = true;

// Set playback time range
control.properties.play_from = "2025-08-15T10:00:00.000Z";
control.properties.play_to = "2025-08-15T12:00:00.000Z";
control.properties.speed = 1.0;
control.properties.play = true;
```

### Recording Control

```javascript
// Start a recording with reason
control.properties.recording_reason = "Security incident";
control.properties.recording_active = true;

// Stop recording
control.properties.recording_active = false;
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
if (control.properties.connected) {
    console.log(`FPS: ${control.properties.fps}`);
    console.log(`Bandwidth: ${control.properties.kbs} KB/s`);
    console.log(`Current time: ${control.properties.timestamp}`);
}
```

## URL Structure

The control automatically constructs the full endpoint URL based on the base URL and control mode:

- **Base URL**: Set via the `URL` property (e.g., `ws://localhost:8080/cam1`)
- **Stream Mode** (`control=false`): Appends `/stream` to base URL
- **Control Mode** (`control=true`): Appends `/control` to base URL

### Additional Endpoints

The base URL approach supports multiple endpoints for different functionality:
- **PTZ Control**: `/camera-path/control/ptz/{command}` (move, stop, goto-preset, set-preset)
- **Configuration**: `/camera-path/config` (planned)

Example usage:
```javascript
// For streaming mode
control.properties.URL = "ws://localhost:8080/cam1";
control.properties.control = false;  // Results in ws://localhost:8080/cam1/stream

// For control mode
control.properties.URL = "ws://localhost:8080/cam1";
control.properties.control = true;   // Results in ws://localhost:8080/cam1/control
```

## Authentication

If your RTSP streaming server requires authentication:

1. Set the `token` property with your access token
2. The control will automatically include it in WebSocket connection headers
3. The token can also be passed as a query parameter

## Troubleshooting

### Enable Debug Mode
```javascript
control.properties.debug = true;
control.properties.version = true;
```

### Common Issues

1. **Connection Fails**: Check URL format and server availability
2. **Authentication Errors**: Verify token is correct and not expired
3. **No Video**: Ensure camera is configured and streaming
4. **Performance Issues**: Monitor `fps` and `kbs` properties

### SSL/TLS Issues

The control automatically handles certificate issues by falling back from WSS to WS connections when necessary.

## Build Information

- **Version**: 1.0.22
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