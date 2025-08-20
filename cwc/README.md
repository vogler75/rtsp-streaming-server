# VideoPlayer Custom Web Control (CWC)

A Siemens WinCC Unified Custom Web Control for video streaming and playback with RTSP streaming server integration.

## Overview

This Custom Web Control provides a video player interface that can connect to RTSP streaming servers via WebSocket connections. It supports both live streaming and recorded video playback with advanced control features.

## Properties

The VideoPlayer CWC exposes the following properties that can be configured and bound in WinCC Unified:

### Connection Properties

#### `URL` (string)
- **Description**: WebSocket connection URL for the video stream
- **Default**: `""` (empty string)
- **Example**: `"ws://localhost:8080/cam1/live"` or `"wss://server.example.com/camera1/stream"`
- **Usage**: Set this to the WebSocket endpoint of your RTSP streaming server

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
- **Usage**: Set to `true` to enable advanced playback controls for recorded video

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

### Performance Monitoring

```javascript
// Monitor connection and performance
if (control.properties.connected) {
    console.log(`FPS: ${control.properties.fps}`);
    console.log(`Bandwidth: ${control.properties.kbs} KB/s`);
    console.log(`Current time: ${control.properties.timestamp}`);
}
```

## URL Endpoints

The control supports different WebSocket endpoints based on usage:

- **Live Streaming**: `/camera-path/live`
- **Stream Viewing**: `/camera-path/stream` 
- **Control Mode**: `/camera-path/control`

Example URLs:
```
ws://localhost:8080/cam1/live
ws://localhost:8080/cam1/control
wss://server.example.com/camera1/stream
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