// Copyright 2022 Siemens AG. This file is subject to the terms and conditions of the MIT License. See LICENSE file in the top-level directory.
//
// SPDX-License-Identifier: MIT

////////////////////////////////////////////
// VideoPlayer internal properties

let websocket = null;
let videoElement = null;
let hlsElement = null;
let hls = null;
let statusElement = null;
let versionElement = null;
let isConnected = false;
let currentCameraUrl = '';
let currentCameraAuthToken = '';
let enableConnection = false;
let reconnectTimer = null;
let reconnectAttempts = 0;
let intentionalClose = false; // Flag to prevent reconnection on intentional close

// FPS and bitrate tracking variables
let frameCount = 0;
let fpsCounter = 0;
let lastFpsTime = Date.now();
let bytesReceived = 0;
let lastBitrateTime = Date.now();

// Control mode variables
let useControlStream = false;
let controlWebSocket = null;
let currentEnablePlaybackbackStartTime = '';
let currentEnablePlaybackbackEndTime = '';
let currentEnablePlayback = false;
let currentEnablePlaybackbackSpeed = 1.0;
let currentEnableLivestream = false;
let currentRecordingReason = '';
let currentEnableRecording = false;
let currentSeekToTime = '';
let enableDebug = false;

// PTZ control variables
let currentPtzMove = '';
let currentPtzStop = false;
let currentPtzGotoPreset = '';
let currentPtzSetPreset = '';

// HLS mode variables
let currentUseHlsStreaming = false;

////////////////////////////////////////////
// Debug helper function
function debugLog(...args) {
  if (enableDebug) {
    console.log('[DEBUG]', ...args);
  }
}

function debugError(...args) {
  if (enableDebug) {
    console.error('[DEBUG]', ...args);
  }
}

function debugWarn(...args) {
  if (enableDebug) {
    console.warn('[DEBUG]', ...args);
  }
}

////////////////////////////////////////////
// VideoPlayer functions

function initializeVideoPlayer() {
  videoElement = document.getElementById('videoPlayer');
  hlsElement = document.getElementById('hlsPlayer');
  statusElement = document.getElementById('status');
  versionElement = document.getElementById('versionDisplay');
  
  videoElement.onerror = function(e) {
    // Only log video errors if we're actually using the video element
    if (videoElement.style.display !== 'none') {
      debugError('Video error:', e);
      updateConnectionStatus(false);
    }
  };
  
  videoElement.onloadstart = function() {
    if (videoElement.style.display !== 'none') {
      debugLog('Video loading started');
    }
  };
  
  videoElement.oncanplay = function() {
    if (videoElement.style.display !== 'none') {
      debugLog('Video can start playing');
    }
  };
}

function updateConnectionStatus(connected) {
  isConnected = connected;
  WebCC.Properties.status_connected = connected;
  
  // Reset statistics when disconnected
  if (!connected) {
    frameCount = 0;
    fpsCounter = 0;
    lastFpsTime = Date.now();
    bytesReceived = 0;
    lastBitrateTime = Date.now();
    WebCC.Properties.status_fps = 0;
    WebCC.Properties.status_bitrate_kbps = 0;
  }
  
  if (statusElement) {
    // In HLS mode, hide status completely unless debug mode is on
    if (currentUseHlsStreaming) {
      if (enableDebug) {
        statusElement.style.display = 'block';
        statusElement.textContent = 'HLS Stream';
        statusElement.style.backgroundColor = 'rgba(0,128,0,0.7)';
      } else {
        statusElement.style.display = 'none';
      }
    } else if (connected) {
      // WebSocket mode - hide status when connected unless debug mode is on
      if (enableDebug) {
        statusElement.style.display = 'block';
        // Show different status based on control mode and livestream state
        if (useControlStream && currentEnableLivestream) {
          statusElement.textContent = 'Live Stream';
        } else if (useControlStream) {
          statusElement.textContent = 'Connected';
        } else {
          statusElement.textContent = 'Live Stream';
        }
        statusElement.style.backgroundColor = 'rgba(0,128,0,0.7)';
      } else {
        statusElement.style.display = 'none';
      }
      reconnectAttempts = 0;
    } else if (enableConnection) {
      // WebSocket mode - show status when disconnected but trying to connect
      statusElement.style.display = 'block';
      if (reconnectAttempts > 0) {
        statusElement.textContent = `Reconnecting in 1s... (${reconnectAttempts})`;
        statusElement.style.backgroundColor = 'rgba(255,165,0,0.7)'; // Orange for reconnecting
      } else {
        statusElement.textContent = 'Disconnected';
        statusElement.style.backgroundColor = 'rgba(128,0,0,0.7)';
      }
    } else {
      // WebSocket mode - show status when stopped
      statusElement.style.display = 'block';
      statusElement.textContent = 'Stopped';
      statusElement.style.backgroundColor = 'rgba(64,64,64,0.7)';
    }
  }
  
  // Connection status updated
}

function scheduleReconnect() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  
  // Don't reconnect if this was an intentional close
  if (intentionalClose) {
    debugLog('Skipping reconnect - connection was intentionally closed');
    intentionalClose = false; // Reset the flag
    return;
  }
  
  // Only reconnect if enableConnection is true AND not in HLS mode
  if (enableConnection && currentCameraUrl && !currentUseHlsStreaming) {
    reconnectAttempts++;
    
    // Simple 1-second delay for all attempts
    const delay = 1000;
    
    debugLog(`Scheduling reconnect attempt ${reconnectAttempts} in ${Math.round(delay/1000)}s`);
    updateConnectionStatus(false);
    
    reconnectTimer = setTimeout(() => {
      if (enableConnection && !currentUseHlsStreaming) { // Check again in case it changed during timeout
        debugLog(`Attempting to reconnect (${reconnectAttempts})`);
        connectToWebSocket(currentCameraUrl);
      }
    }, delay);
  } else {
    debugLog('Not reconnecting: enableConnection =', enableConnection, 'currentCameraUrl =', currentCameraUrl, 'currentUseHlsStreaming =', currentUseHlsStreaming);
  }
}

function showBlankScreen() {
  // Hide video/image elements
  videoElement.style.display = 'none';
  hlsElement.style.display = 'none';
  const imgElement = document.getElementById('mjpegFrame');
  if (imgElement) {
    imgElement.style.display = 'none';
    // Clean up any blob URL
    if (imgElement.src && imgElement.src.startsWith('blob:')) {
      URL.revokeObjectURL(imgElement.src);
    }
  }
  // Clean up HLS instance
  if (hls) {
    hls.destroy();
    hls = null;
  }
}

function switchPlayerMode() {
  if (currentUseHlsStreaming) {
    // Show HLS player, hide WebSocket player
    videoElement.style.display = 'none';
    hlsElement.style.display = 'block';
    const imgElement = document.getElementById('mjpegFrame');
    if (imgElement) {
      imgElement.style.display = 'none';
    }
    
    // Cancel any pending reconnect attempts
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
      debugLog('Cancelled pending reconnect timer for HLS mode');
    }
    
    // Disconnect WebSocket if connected
    intentionalClose = true; // Prevent automatic reconnection
    if (websocket) {
      // Remove all event handlers first
      websocket.onopen = null;
      websocket.onmessage = null;
      websocket.onerror = null;
      websocket.onclose = null;
      websocket.close();
      websocket = null;
    }
    if (controlWebSocket) {
      // Remove all event handlers first
      controlWebSocket.onopen = null;
      controlWebSocket.onmessage = null;
      controlWebSocket.onerror = null;
      controlWebSocket.onclose = null;
      controlWebSocket.close();
      controlWebSocket = null;
    }
    
    // Reset connection status without triggering reconnect
    updateConnectionStatus(false);
    debugLog('Switched to HLS player mode');
  } else {
    // Show WebSocket player, hide HLS player
    videoElement.style.display = 'block';
    hlsElement.style.display = 'none';
    
    // Clean up HLS
    if (hls) {
      hls.destroy();
      hls = null;
    }
    
    debugLog('Switched to WebSocket player mode');
    
    // Reset reconnect attempts when switching back to WebSocket mode
    reconnectAttempts = 0;
    
    // Reconnect WebSocket if needed
    if (enableConnection && currentCameraUrl) {
      connectToWebSocket(currentCameraUrl);
    }
  }
}

function connectToWebSocket(url) {
  if (!url || url.trim() === '' || !enableConnection || currentUseHlsStreaming) {
    debugLog('No URL provided, connection disabled, or in HLS mode');
    updateConnectionStatus(false);
    return;
  }
  
  // Build the full URL based on control mode
  // URL should be base path like /cam1, we append /stream or /control
  let fullUrl = url;
  
  // Remove any trailing slash
  if (fullUrl.endsWith('/')) {
    fullUrl = fullUrl.slice(0, -1);
  }
  
  // Check if we're in control mode and need to connect to control endpoint
  if (useControlStream) {
    fullUrl = fullUrl + '/control';
    connectToControlWebSocket(fullUrl);
    return;
  } else {
    fullUrl = fullUrl + '/stream';
  }
  
  // Close existing WebSocket if it exists (prevent duplicate connections)
  if (websocket) {
    // Remove all event handlers first to prevent them from firing after we create a new connection
    websocket.onopen = null;
    websocket.onmessage = null;
    websocket.onerror = null;
    websocket.onclose = null;
    websocket.close();
    websocket = null;
  }
  
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  
  // Reset intentional close flag - we want this new connection to be able to reconnect
  intentionalClose = false;
  
  try {
    debugLog('Attempting WebSocket connection...');
    debugLog('URL:', fullUrl);
    debugLog('Protocol:', fullUrl.startsWith('wss://') ? 'Secure WebSocket (WSS)' : 'WebSocket (WS)');
    
    // Add token to URL if provided
    let connectUrl = fullUrl;
    if (currentCameraAuthToken && currentCameraAuthToken.trim() !== '') {
      const separator = fullUrl.includes('?') ? '&' : '?';
      connectUrl = fullUrl + separator + 'token=' + encodeURIComponent(currentCameraAuthToken);
      debugLog('Using authentication token');
    }
    
    // For WSS with self-signed certificates, try different approaches
    if (connectUrl.startsWith('wss://')) {
      debugLog('Attempting WSS connection (ignoring certificate errors where possible)');
      
      // Try to create WebSocket with additional error handling for certificate issues
      try {
        websocket = new WebSocket(connectUrl);
      } catch (certError) {
        debugWarn('WSS connection failed, possibly due to certificate issues:', certError);
        
        // Fallback: try converting WSS to WS for testing
        const wsUrl = connectUrl.replace('wss://', 'ws://');
        debugLog('Attempting fallback to WS:', wsUrl);
        websocket = new WebSocket(wsUrl);
      }
    } else {
      websocket = new WebSocket(connectUrl);
    }
    
    websocket.onopen = function() {
      debugLog('WebSocket connected');
      intentionalClose = false; // Reset flag on successful connection
      updateConnectionStatus(true);
    };
    
    websocket.onmessage = function(event) {
      if (event.data instanceof Blob) {
        // Check if this is an empty frame (ping) - same as server HTML
        if (event.data.size === 0) {
          return;
        }
        
        // Handle MJPEG frame from WebSocket
        // Create or update an img element to display the MJPEG frame
        let imgElement = document.getElementById('mjpegFrame');
        if (!imgElement) {
          imgElement = document.createElement('img');
          imgElement.id = 'mjpegFrame';
          imgElement.style.width = '100%';
          imgElement.style.height = '100%';
          imgElement.style.objectFit = 'contain';
          
          // Replace video element with img element
          videoElement.style.display = 'none';
          videoElement.parentNode.insertBefore(imgElement, videoElement.nextSibling);
        } else {
          // Make sure img element is visible (might have been hidden by showBlankScreen)
          imgElement.style.display = 'block';
          videoElement.style.display = 'none';
        }
        
        // Create blob URL and clean up previous one
        const previousSrc = imgElement.src;
        if (previousSrc && previousSrc.startsWith('blob:')) {
          URL.revokeObjectURL(previousSrc);
        }
        
        // Set new blob URL
        const blobUrl = URL.createObjectURL(event.data);
        imgElement.src = blobUrl;
        
        // Update statistics
        frameCount++;
        fpsCounter++;
        bytesReceived += event.data.size;
        
        const now = Date.now();
        
        // FPS calculation - update every second
        if (now - lastFpsTime >= 1000) {
          const fps = Math.round(fpsCounter * 1000 / (now - lastFpsTime));
          WebCC.Properties.status_fps = fps;
          fpsCounter = 0;
          lastFpsTime = now;
        }
        
        // Bitrate calculation - update every second
        if (now - lastBitrateTime >= 1000) {
          const kbs = Math.round(bytesReceived / 1024); // KB/s
          WebCC.Properties.status_bitrate_kbps = kbs;
          bytesReceived = 0;
          lastBitrateTime = now;
        }
        
        // Clean up blob URL after image loads
        imgElement.onload = function() {
          // Small delay before cleanup to ensure image is displayed
          setTimeout(() => {
            URL.revokeObjectURL(blobUrl);
          }, 100);
        };        
      } 
    };
    
    websocket.onerror = function(error) {
      debugError('WebSocket error occurred');
      debugError('URL:', fullUrl);
      debugError('ReadyState:', websocket ? websocket.readyState : 'null');
      debugError('Error event:', error);
      
      // Provide specific guidance for WSS certificate issues
      if (fullUrl.startsWith('wss://')) {
        debugError('====== WSS CONNECTION TROUBLESHOOTING ======');
        debugError('If this is a self-signed certificate error:');
        debugError('1. Open the server URL in a browser: ' + fullUrl.replace('wss://', 'https://'));
        debugError('2. Accept the security warning to trust the certificate');
        debugError('3. Or disable TLS in server config.json and use ws:// instead');
        debugError('4. Or add the certificate to the system trust store');
        debugError('============================================');
      }
      
      updateConnectionStatus(false);
      scheduleReconnect();
    };
    
    websocket.onclose = function(event) {
      debugLog('WebSocket closed');
      debugLog('Code:', event.code);
      debugLog('Reason:', event.reason || 'No reason provided');
      debugLog('Was clean:', event.wasClean);
      debugLog('URL was:', fullUrl);
      
      // Common WebSocket close codes
      const closeReasons = {
        1000: 'Normal Closure',
        1001: 'Going Away',
        1002: 'Protocol Error',
        1003: 'Unsupported Data',
        1006: 'Abnormal Closure',
        1007: 'Invalid frame payload data',
        1008: 'Policy Violation',
        1009: 'Message too big',
        1010: 'Missing Extension',
        1011: 'Internal Error',
        1015: 'TLS Handshake'
      };
      
      const closeDescription = closeReasons[event.code] || 'Unknown';
      debugLog('Close description:', closeDescription);
      
      updateConnectionStatus(false);
      websocket = null;
      
      // Check for authentication failures - don't retry
      if (event.code === 1002 || event.code === 1003) {
        debugLog('Authentication failed - not attempting reconnection');
        return;
      }
      
      if (event.code !== 1000) {
        scheduleReconnect();
      }
    };
    
  } catch (error) {
    debugError('Failed to create WebSocket:', error);
    updateConnectionStatus(false);
    scheduleReconnect();
  }
}


function connectToControlWebSocket(url) {
  debugLog('🔗 Connecting to control WebSocket...');
  
  if (controlWebSocket) {
    debugLog('Closing existing control WebSocket');
    // Remove all event handlers first
    controlWebSocket.onopen = null;
    controlWebSocket.onmessage = null;
    controlWebSocket.onerror = null;
    controlWebSocket.onclose = null;
    controlWebSocket.close();
    controlWebSocket = null;
  }
  
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  
  // Reset intentional close flag - we want this new connection to be able to reconnect
  intentionalClose = false;
  
  try {
    // URL already has /control appended by connectToWebSocket
    let controlUrl = url;
    
    // Only add token if provided - no other parameters
    if (currentCameraAuthToken && currentCameraAuthToken.trim() !== '') {
      const separator = controlUrl.includes('?') ? '&' : '?';
      controlUrl += separator + 'token=' + encodeURIComponent(currentCameraAuthToken);
      debugLog('Adding token to control URL');
    }
    
    debugLog('🔗 Control URL:', controlUrl);
    controlWebSocket = new WebSocket(controlUrl);
    
    controlWebSocket.onopen = function() {
      debugLog('✅ Control WebSocket connected successfully');
      intentionalClose = false; // Reset flag on successful connection
      updateConnectionStatus(true);
      
      // If livestream is already enabled when we connect, start it immediately
      if (currentEnableLivestream) {
        debugLog('📺 Auto-starting live stream (was already enabled)');
        sendControlCommand({ cmd: 'live' });
      }
    };
    
    controlWebSocket.onmessage = function(event) {
      if (event.data instanceof Blob) {
        // Handle binary messages from control WebSocket
        if (event.data.size === 0) {
          return; // Skip empty frames
        }
        
        // Read the binary data to check protocol byte
        event.data.arrayBuffer().then(buffer => {
          const dataView = new DataView(buffer);
          
          if (buffer.byteLength < 9) {
            debugWarn('Control message too short, expected at least 9 bytes');
            return;
          }
          
          // First byte indicates message type
          const messageType = dataView.getUint8(0);
          
          if (messageType === 0) {
            // Video frame message
            // Next 8 bytes are timestamp (little-endian 64-bit integer)
            const timestampMs = dataView.getBigInt64(1, true); // little-endian
            const frameData = buffer.slice(9); // Rest is frame data
            
            // Update the timestamp property with the current frame timestamp in ISO format
            const timestampNumber = Number(timestampMs);
            const timestampISO = new Date(timestampNumber).toISOString();
            WebCC.Properties.status_timestamp = timestampISO;
            
            // Video frame received and timestamp property updated (removed verbose logging)
            
            // Create or update an img element to display the frame
            let imgElement = document.getElementById('mjpegFrame');
            if (!imgElement) {
              imgElement = document.createElement('img');
              imgElement.id = 'mjpegFrame';
              imgElement.style.width = '100%';
              imgElement.style.height = '100%';
              imgElement.style.objectFit = 'contain';
              
              videoElement.style.display = 'none';
              videoElement.parentNode.insertBefore(imgElement, videoElement.nextSibling);
            } else {
              imgElement.style.display = 'block';
              videoElement.style.display = 'none';
            }
            
            // Create blob URL and clean up previous one
            const previousSrc = imgElement.src;
            if (previousSrc && previousSrc.startsWith('blob:')) {
              URL.revokeObjectURL(previousSrc);
            }
            
            const frameBlob = new Blob([frameData], { type: 'image/jpeg' });
            const blobUrl = URL.createObjectURL(frameBlob);
            imgElement.src = blobUrl;
            
            // Update statistics
            frameCount++;
            fpsCounter++;
            bytesReceived += event.data.size;
            
            const now = Date.now();
            
            // FPS calculation
            if (now - lastFpsTime >= 1000) {
              const fps = Math.round(fpsCounter * 1000 / (now - lastFpsTime));
              WebCC.Properties.status_fps = fps;
              fpsCounter = 0;
              lastFpsTime = now;
            }
            
            // Bitrate calculation
            if (now - lastBitrateTime >= 1000) {
              const kbs = Math.round(bytesReceived / 1024);
              WebCC.Properties.status_bitrate_kbps = kbs;
              bytesReceived = 0;
              lastBitrateTime = now;
            }
            
            // Clean up blob URL after image loads
            imgElement.onload = function() {
              setTimeout(() => {
                URL.revokeObjectURL(blobUrl);
              }, 100);
            };
            
          } else if (messageType === 1) {
            // JSON response message
            const jsonData = buffer.slice(1); // Skip first byte
            const jsonString = new TextDecoder().decode(jsonData);
            
            try {
              const response = JSON.parse(jsonString);
              debugLog('⬅ RECEIVED JSON RESPONSE:', response);
              
              // Handle command responses
              if (response.code) {
                if (response.code === 200) {
                  debugLog('✅ Control command successful:', response.text);
                  if (response.data) {
                    debugLog('Response data:', response.data);
                  }
                } else {
                  debugError('❌ Control command failed:', response.text, 'Code:', response.code);
                }
              }
            } catch (e) {
              debugError('⬅ Failed to parse JSON response:', e);
              debugLog('Raw JSON data:', jsonString);
            }
          } else {
            debugWarn('⬅ UNKNOWN MESSAGE TYPE:', messageType, 'Buffer size:', buffer.byteLength);
          }
        }).catch(error => {
          debugError('⬅ Error reading binary data:', error);
        });
        
      } else if (typeof event.data === 'string') {
        // Handle text messages (fallback)
        try {
          const response = JSON.parse(event.data);
          debugLog('⬅ RECEIVED TEXT RESPONSE:', response);
          
          // Handle command responses
          if (response.code) {
            if (response.code === 200) {
              debugLog('✅ Control command successful (text):', response.text);
            } else {
              debugError('❌ Control command failed (text):', response.text);
            }
          }
        } catch (e) {
          debugLog('⬅ RECEIVED TEXT MESSAGE (non-JSON):', event.data);
        }
      }
    };
    
    controlWebSocket.onerror = function(error) {
      debugError('❌ Control WebSocket error:', error);
      updateConnectionStatus(false);
      scheduleReconnect();
    };
    
    controlWebSocket.onclose = function(event) {
      debugLog('🔌 Control WebSocket closed:', event.code, event.reason || 'No reason provided');
      updateConnectionStatus(false);
      controlWebSocket = null;
      
      // Check for authentication failures - don't retry
      if (event.code === 1002 || event.code === 1003) {
        debugLog('Authentication failed - not attempting reconnection');
        return;
      }
      
      if (event.code !== 1000) {
        debugLog('Scheduling reconnect due to abnormal close');
        scheduleReconnect();
      }
    };
    
  } catch (error) {
    debugError('❌ Failed to create control WebSocket:', error);
    updateConnectionStatus(false);
    scheduleReconnect();
  }
}

function sendControlCommand(command) {
  if (!controlWebSocket) return;
  
  if (controlWebSocket.readyState !== WebSocket.OPEN) {
    debugError('Control WebSocket not connected - cannot send command:', command);
    return;
  }
  
  const commandStr = JSON.stringify(command);
  debugLog('➤ SENDING CONTROL COMMAND:', commandStr);
  controlWebSocket.send(commandStr);
}

function handleRecordingControl(active, reason) {
  if (!useControlStream || !currentCameraUrl) {
    debugLog('Recording control only available in control mode');
    return;
  }
  
  // Use HTTP API for recording control
  // Convert WebSocket URL to HTTP URL
  // currentCameraUrl is now just the base path like /cam1
  let baseUrl = currentCameraUrl.replace(/^ws(s?):\/\//, 'http$1://');
  
  // Build the full endpoint URL
  const endpoint = active ? '/control/recording/start' : '/control/recording/stop';
  const fullUrl = baseUrl + endpoint;
  
  const requestOptions = {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json'
    }
  };
  
  if (currentCameraAuthToken) {
    requestOptions.headers['Authorization'] = 'Bearer ' + currentCameraAuthToken;
    debugLog('Using token authentication for HTTP request');
  }
  
  if (active) {
    // Always pass reason when starting recording, use empty string if no reason provided
    requestOptions.body = JSON.stringify({ reason: reason || '' });
  }
  
  debugLog('➤ SENDING HTTP REQUEST:', {
    method: 'POST',
    url: fullUrl,
    body: requestOptions.body || '(no body)',
    headers: requestOptions.headers
  });
  
  fetch(fullUrl, requestOptions)
    .then(response => {
      debugLog('⬅ HTTP RESPONSE STATUS:', response.status, response.statusText);
      return response.json();
    })
    .then(data => {
      debugLog('⬅ HTTP RESPONSE DATA:', data);
    })
    .catch(error => {
      debugError('⬅ HTTP REQUEST ERROR:', error);
    });
}

function handlePtzControl(endpoint, jsonData) {
  if (!currentCameraUrl) {
    debugLog('PTZ control requires URL to be set');
    return;
  }
  
  // Convert WebSocket URL to HTTP URL
  // currentCameraUrl is now just the base path like /cam1
  let baseUrl = currentCameraUrl.replace(/^ws(s?):\/\//, 'http$1://');
  
  // Build the full PTZ endpoint URL
  const fullUrl = baseUrl + '/control/ptz/' + endpoint;
  
  const requestOptions = {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json'
    }
  };
  
  if (currentCameraAuthToken) {
    requestOptions.headers['Authorization'] = 'Bearer ' + currentCameraAuthToken;
    debugLog('Using token authentication for PTZ request');
  }
  
  if (jsonData) {
    requestOptions.body = jsonData;
  }
  
  debugLog('🎯 SENDING PTZ REQUEST:', {
    method: 'POST',
    url: fullUrl,
    body: requestOptions.body || '(no body)',
    headers: requestOptions.headers
  });
  
  fetch(fullUrl, requestOptions)
    .then(response => {
      debugLog('⬅ PTZ RESPONSE STATUS:', response.status, response.statusText);
      
      // Check if response is successful
      if (response.ok) {
        // Try to parse as JSON, but handle plain text responses
        return response.text().then(text => {
          try {
            return JSON.parse(text);
          } catch (e) {
            // If it's not JSON, return the text as is
            return { message: text };
          }
        });
      } else {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      }
    })
    .then(data => {
      debugLog('⬅ PTZ RESPONSE DATA:', data);
    })
    .catch(error => {
      debugError('⬅ PTZ REQUEST ERROR:', error);
    });
}

function buildHlsUrl(cameraPath, fromTimestamp, toTimestamp) {
  // Build HLS URL using the new camera path structure
  const t1 = encodeURIComponent(fromTimestamp);
  const t2 = encodeURIComponent(toTimestamp);
  
  // Use the camera path directly for the new endpoint structure
  let cleanPath = cameraPath;
  if (cameraPath.startsWith('http')) {
    // Extract just the path from full URL
    const url = new URL(cameraPath);
    cleanPath = url.pathname;
  }
  
  debugLog('Building HLS URL for camera path:', cleanPath, 'from:', fromTimestamp, 'to:', toTimestamp);
  return `${cleanPath}/control/recordings/hls/timerange?t1=${t1}&t2=${t2}`;
}

function playHlsStream(url) {
  if (!hlsElement) {
    debugError('HLS element not found');
    return;
  }
  
  debugLog('Playing HLS stream:', url);
  
  // Debug: Fetch and log the HLS playlist content
  debugHlsPlaylist(url);
  
  // Clean up any existing HLS instance
  if (hls) {
    hls.destroy();
    hls = null;
  }
  
  if (Hls.isSupported()) {
    // Use HLS.js for browsers that don't support HLS natively
    hls = new Hls({
      debug: enableDebug,
      enableWorker: true
    });
    
    hls.loadSource(url);
    hls.attachMedia(hlsElement);
    
    hls.on(Hls.Events.MANIFEST_PARSED, function() {
      debugLog('HLS: Manifest parsed successfully');
      hlsElement.play().catch(function(error) {
        debugError('Failed to play HLS:', error.message);
      });
    });
    
    hls.on(Hls.Events.ERROR, function(event, data) {
      debugError('HLS Error:', data.type, '-', data.details);
      if (data.fatal) {
        switch(data.type) {
          case Hls.ErrorTypes.NETWORK_ERROR:
            debugError('HLS: Fatal network error - trying to recover');
            hls.startLoad();
            break;
          case Hls.ErrorTypes.MEDIA_ERROR:
            debugError('HLS: Fatal media error - trying to recover');
            hls.recoverMediaError();
            break;
          default:
            debugError('HLS: Fatal error, destroying HLS instance');
            hls.destroy();
            hls = null;
            break;
        }
      }
    });
    
    hls.on(Hls.Events.LEVEL_LOADED, function(event, data) {
      debugLog('HLS: Level loaded -', data.details, 'segments');
    });
    
  } else if (hlsElement.canPlayType('application/vnd.apple.mpegurl')) {
    // Native HLS support (Safari)
    debugLog('Using native HLS support');
    hlsElement.src = url;
    hlsElement.play().catch(function(error) {
      debugError('Failed to play HLS natively:', error.message);
    });
  } else {
    debugError('HLS not supported in this browser');
  }
}

function setProperty(data) {  
  switch (data.key) {
    case 'camera_stream_url':
      if (data.value !== currentCameraUrl) {
        const oldURL = currentCameraUrl;
        currentCameraUrl = data.value;
        reconnectAttempts = 0;
        
        // ALWAYS close existing connections first when URL changes
        debugLog('URL changed from', oldURL, 'to', currentCameraUrl, '- closing any existing connections...');
        
        // Close existing connections when URL changes
        if (websocket) {
          // Remove all event handlers first
          websocket.onopen = null;
          websocket.onmessage = null;
          websocket.onerror = null;
          websocket.onclose = null;
          websocket.close();
          websocket = null;
        }
        
        // Close control WebSocket connection if it exists
        if (controlWebSocket) {
          // Remove all event handlers first
          controlWebSocket.onopen = null;
          controlWebSocket.onmessage = null;
          controlWebSocket.onerror = null;
          controlWebSocket.onclose = null;
          controlWebSocket.close();
          controlWebSocket = null;
        }
        
        // Clear any pending reconnection timers
        if (reconnectTimer) {
          clearTimeout(reconnectTimer);
          reconnectTimer = null;
        }
        
        // Now reconnect if connection is enabled (connectToWebSocket will reset intentionalClose)
        if (enableConnection) {
          if (currentCameraUrl) {
            connectToWebSocket(currentCameraUrl);
          } else {
            // No URL provided, just update status
            updateConnectionStatus(false);
            showBlankScreen();
          }
        } else {
          // Connection disabled, just update status
          updateConnectionStatus(false);
          showBlankScreen();
        }
      }
      break;
    case 'enable_connection':
      enableConnection = data.value;
      if (enableConnection && !currentUseHlsStreaming) {
        // Start connecting (only if not in HLS mode)
        reconnectAttempts = 0;
        if (currentCameraUrl) {
          connectToWebSocket(currentCameraUrl);
        }
      } else if (enableConnection && currentUseHlsStreaming) {
        debugLog('Connect requested but in HLS mode - ignoring WebSocket connection');
      } else {
        // Disconnect and show blank screen
        intentionalClose = true; // Prevent automatic reconnection
        if (websocket) {
          // Remove all event handlers first
          websocket.onopen = null;
          websocket.onmessage = null;
          websocket.onerror = null;
          websocket.onclose = null;
          websocket.close();
          websocket = null;
        }
        if (controlWebSocket) {
          // Remove all event handlers first
          controlWebSocket.onopen = null;
          controlWebSocket.onmessage = null;
          controlWebSocket.onerror = null;
          controlWebSocket.onclose = null;
          controlWebSocket.close();
          controlWebSocket = null;
        }
        if (reconnectTimer) {
          clearTimeout(reconnectTimer);
          reconnectTimer = null;
        }
        updateConnectionStatus(false);
        showBlankScreen();
      }
      break;
    case 'show_version':
      if (versionElement) {
        versionElement.style.display = data.value ? 'block' : 'none';
      }
      break;
    case 'camera_auth_token':
      if (data.value !== currentCameraAuthToken) {
        currentCameraAuthToken = data.value;
        debugLog('Token updated');
        
        // If we're connected and token changed, reconnect with new token
        if (enableConnection && currentCameraUrl) {
          if (websocket) {
            debugLog('Token changed - reconnecting with new authentication...');
            // Remove all event handlers first
            websocket.onopen = null;
            websocket.onmessage = null;
            websocket.onerror = null;
            websocket.onclose = null;
            websocket.close();
            websocket = null;
          }
          
          // Clear any pending reconnection timers
          if (reconnectTimer) {
            clearTimeout(reconnectTimer);
            reconnectTimer = null;
          }
          
          // Reconnect with new token (connectToWebSocket will reset intentionalClose)
          connectToWebSocket(currentCameraUrl);
        }
      }
      break;
    case 'use_control_stream':
      useControlStream = data.value;
      debugLog('🔧 Control mode changed to:', useControlStream);
      
      // Update status display when control mode changes
      updateConnectionStatus(isConnected);
      
      // Reconnect if URL is available and we're supposed to be connected
      if (enableConnection && currentCameraUrl) {
        debugLog('Reconnecting due to control mode change...');
        if (websocket) {
          // Remove all event handlers first
          websocket.onopen = null;
          websocket.onmessage = null;
          websocket.onerror = null;
          websocket.onclose = null;
          websocket.close();
          websocket = null;
        }
        if (controlWebSocket) {
          // Remove all event handlers first
          controlWebSocket.onopen = null;
          controlWebSocket.onmessage = null;
          controlWebSocket.onerror = null;
          controlWebSocket.onclose = null;
          controlWebSocket.close();
          controlWebSocket = null;
        }
        // connectToWebSocket will reset intentionalClose flag
        connectToWebSocket(currentCameraUrl);
      }
      break;
    case 'playback_start_time':
      currentEnablePlaybackbackStartTime = data.value;
      debugLog('🔧 Play from timestamp changed to:', currentEnablePlaybackbackStartTime);
      break;
    case 'playback_end_time':
      currentEnablePlaybackbackEndTime = data.value;
      debugLog('🔧 Play to timestamp changed to:', currentEnablePlaybackbackEndTime);
      break;
    case 'enable_playback':
      currentEnablePlayback = data.value;
      debugLog('🔧 Play control changed to:', currentEnablePlayback);
      
      if (currentUseHlsStreaming) {
        // HLS mode playback
        if (currentEnablePlayback && currentEnablePlaybackbackStartTime && currentCameraUrl) {
          const toTime = currentEnablePlaybackbackEndTime || new Date().toISOString();
          const hlsUrl = buildHlsUrl(currentCameraUrl, currentEnablePlaybackbackStartTime, toTime);
          debugLog('🎬 Starting HLS playback from:', currentEnablePlaybackbackStartTime, 'to:', toTime);
          playHlsStream(hlsUrl);
        } else if (!currentEnablePlayback) {
          debugLog('⏹️ Stopping HLS playback');
          if (hls) {
            hls.destroy();
            hls = null;
          }
          if (hlsElement) {
            hlsElement.pause();
            hlsElement.currentTime = 0;
          }
        } else {
          debugWarn('⚠️ HLS playback ignored - missing required parameters (playback_start_time or URL)');
        }
      } else if (useControlStream && controlWebSocket) {
        // WebSocket control mode playback
        if (currentEnablePlayback) {
          // Start playback
          const command = {
            cmd: 'start',
            from: currentEnablePlaybackbackStartTime
          };
          if (currentEnablePlaybackbackEndTime) {
            command.to = currentEnablePlaybackbackEndTime;
          }
          debugLog('🎬 Triggering playback start with timestamps from:', currentEnablePlaybackbackStartTime, 'to:', currentEnablePlaybackbackEndTime || '(end)');
          sendControlCommand(command);
        } else {
          // Stop playback
          debugLog('⏹️ Triggering playback stop');
          sendControlCommand({ cmd: 'stop' });
        }
      } else {
        debugWarn('⚠️ Play command ignored - not in control mode or WebSocket not connected');
      }
      break;
    case 'playback_speed':
      currentPlaybackSpeed = data.value;
      debugLog('🔧 Playback speed changed to:', currentPlaybackSpeed);
      
      if (useControlStream && controlWebSocket) {
        debugLog('⚡ Triggering speed change');
        sendControlCommand({
          cmd: 'speed',
          speed: currentPlaybackSpeed
        });
      } else {
        debugWarn('⚠️ Speed command ignored - not in control mode or WebSocket not connected');
      }
      break;
    case 'enable_livestream':
      currentEnableLivestream = data.value;
      debugLog('🔧 Live stream control changed to:', currentEnableLivestream);
      
      // Update status display when livestream state changes
      updateConnectionStatus(isConnected);
      
      if (useControlStream && controlWebSocket) {
        if (currentEnableLivestream) {
          debugLog('📺 Triggering live stream start');
          sendControlCommand({ cmd: 'live' });
        } else {
          debugLog('⏹️ Triggering live stream stop');
          sendControlCommand({ cmd: 'stop' });
        }
      } else {
        debugWarn('⚠️ Live command ignored - not in control mode or WebSocket not connected');
      }
      break;
    case 'recording_reason':
      currentRecordingReason = data.value;
      debugLog('🔧 Recording reason changed to:', currentRecordingReason);
      break;
    case 'enable_recording':
      currentEnableRecording = data.value;
      debugLog('🔧 Recording active changed to:', currentEnableRecording);
      
      if (useControlStream) {
        debugLog('🔴 Triggering recording control - active:', currentEnableRecording, 'reason:', currentRecordingReason);
        handleRecordingControl(currentEnableRecording, currentRecordingReason);
      } else {
        debugWarn('⚠️ Recording command ignored - not in control mode');
      }
      break;
    case 'seek_to_time':
      currentSeekToTime = data.value;
      debugLog('🔧 Goto timestamp changed to:', currentSeekToTime);
      
      if (useControlStream && controlWebSocket && currentSeekToTime) {
        debugLog('🎯 Triggering goto command to timestamp:', currentSeekToTime);
        sendControlCommand({
          cmd: 'goto',
          timestamp: currentSeekToTime
        });
      } else if (!currentSeekToTime) {
        debugWarn('⚠️ Goto command ignored - empty timestamp');
      } else {
        debugWarn('⚠️ Goto command ignored - not in control mode or WebSocket not connected');
      }
      break;
    case 'enable_debug':
      enableDebug = data.value;
      console.log('Debug logging:', enableDebug ? 'enabled' : 'disabled');
      // Update status display based on debug mode change
      updateConnectionStatus(isConnected);
      break;
    case 'ptz_move':
      currentPtzMove = data.value;
      debugLog('🎯 PTZ move command changed to:', currentPtzMove);
      
      if (currentPtzMove && currentPtzMove.trim() !== '') {
        try {
          // Validate JSON format
          JSON.parse(currentPtzMove);
          debugLog('🎯 Triggering PTZ move command');
          handlePtzControl('move', currentPtzMove);
        } catch (e) {
          debugError('❌ Invalid JSON in ptz_move:', e, 'Value:', currentPtzMove);
        }
      } else {
        debugWarn('⚠️ PTZ move command ignored - empty value');
      }
      break;
    case 'ptz_stop':
      currentPtzStop = data.value;
      debugLog('🎯 PTZ stop changed to:', currentPtzStop);
      
      if (currentPtzStop) {
        debugLog('🛑 Triggering PTZ stop command');
        handlePtzControl('stop', null);
        // Reset the property back to false after sending
        WebCC.Properties.ptz_stop = false;
      }
      break;
    case 'ptz_goto_preset':
      currentPtzGotoPreset = data.value;
      debugLog('🎯 PTZ goto preset changed to:', currentPtzGotoPreset);
      
      if (currentPtzGotoPreset && currentPtzGotoPreset.trim() !== '') {
        try {
          // Validate JSON format
          JSON.parse(currentPtzGotoPreset);
          debugLog('🎯 Triggering PTZ goto preset command');
          handlePtzControl('goto-preset', currentPtzGotoPreset);
        } catch (e) {
          debugError('❌ Invalid JSON in ptz_goto_preset:', e, 'Value:', currentPtzGotoPreset);
        }
      } else {
        debugWarn('⚠️ PTZ goto preset command ignored - empty value');
      }
      break;
    case 'ptz_set_preset':
      currentPtzSetPreset = data.value;
      debugLog('🎯 PTZ set preset changed to:', currentPtzSetPreset);
      
      if (currentPtzSetPreset && currentPtzSetPreset.trim() !== '') {
        try {
          // Validate JSON format
          JSON.parse(currentPtzSetPreset);
          debugLog('🎯 Triggering PTZ set preset command');
          handlePtzControl('set-preset', currentPtzSetPreset);
        } catch (e) {
          debugError('❌ Invalid JSON in ptz_set_preset:', e, 'Value:', currentPtzSetPreset);
        }
      } else {
        debugWarn('⚠️ PTZ set preset command ignored - empty value');
      }
      break;
    case 'use_hls_streaming':
      currentUseHlsStreaming = data.value;
      debugLog('🎬 HLS mode changed to:', currentUseHlsStreaming);
      switchPlayerMode();
      // Update status display based on new mode
      updateConnectionStatus(isConnected);
      break;
  }
}

////////////////////////////////////////////
// Initialize the custom control
// Debug function to fetch and log HLS playlist content
function debugHlsPlaylist(url) {
  if (!enableDebug) return;
  
  debugLog('🔍 Fetching HLS playlist for debugging:', url);
  
  fetch(url)
    .then(response => {
      debugLog('📥 HLS Playlist Response:', response.status, response.statusText);
      debugLog('📋 HLS Content-Type:', response.headers.get('content-type'));
      
      if (response.ok) {
        return response.text();
      } else {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      }
    })
    .then(playlistContent => {
      debugLog('📄 HLS Playlist Content:');
      debugLog('════════════════════════════════════════');
      debugLog(playlistContent);
      debugLog('════════════════════════════════════════');
      
      // Parse and analyze the playlist
      const lines = playlistContent.split('\n');
      const segments = lines.filter(line => line.trim().endsWith('.ts'));
      const duration = lines.find(line => line.startsWith('#EXT-X-TARGETDURATION:'));
      
      debugLog('📊 HLS Playlist Analysis:');
      debugLog('- Total segments:', segments.length);
      debugLog('- Target duration:', duration || 'Not found');
      debugLog('- Playlist size:', playlistContent.length, 'characters');
      
      // Log first few segment URLs for debugging
      if (segments.length > 0) {
        debugLog('🎬 Segment URLs:');
        segments.slice(0, 3).forEach((segment, index) => {
          debugLog(`  ${index + 1}. ${segment}`);
        });
        if (segments.length > 3) {
          debugLog(`  ... and ${segments.length - 3} more segments`);
        }
      }
    })
    .catch(error => {
      debugError('❌ Failed to fetch HLS playlist:', error.message);
      debugError('   URL was:', url);
    });
}

WebCC.start(
  function (result) {
    if (result) {
      console.log('WebCC: Connected successfully');
      initializeVideoPlayer();
      
      // Set initial properties
      currentCameraUrl = WebCC.Properties.camera_stream_url || '';
      currentCameraAuthToken = WebCC.Properties.camera_auth_token || '';
      enableConnection = WebCC.Properties.enable_connection || false;
      useControlStream = WebCC.Properties.use_control_stream || false;
      currentPlaybackStartTime = WebCC.Properties.playback_start_time || '';
      currentPlaybackEndTime = WebCC.Properties.playback_end_time || '';
      currentEnablePlayback = WebCC.Properties.enable_playback || false;
      currentPlaybackSpeed = WebCC.Properties.playback_speed || 1.0;
      currentEnableLivestream = WebCC.Properties.enable_livestream || false;
      currentRecordingReason = WebCC.Properties.recording_reason || '';
      currentEnableRecording = WebCC.Properties.enable_recording || false;
      currentSeekToTime = WebCC.Properties.seek_to_time || '';
      enableDebug = WebCC.Properties.enable_debug || false;
      
      // Initialize PTZ properties
      currentPtzMove = WebCC.Properties.ptz_move || '';
      currentPtzStop = WebCC.Properties.ptz_stop || false;
      currentPtzGotoPreset = WebCC.Properties.ptz_goto_preset || '';
      currentPtzSetPreset = WebCC.Properties.ptz_set_preset || '';
      
      // Initialize HLS property
      currentUseHlsStreaming = WebCC.Properties.use_hls_streaming || false;
      debugLog('🔧 Initial HLS streaming value:', WebCC.Properties.use_hls_streaming, '-> currentUseHlsStreaming:', currentUseHlsStreaming);
      
      // Check version property at startup
      if (versionElement && WebCC.Properties.show_version) {
        versionElement.style.display = 'block';
      }
      
      // Initialize player mode - ensure correct video element is visible
      debugLog('🔧 About to call switchPlayerMode with currentUseHlsStreaming:', currentUseHlsStreaming);
      switchPlayerMode();
      
      // Connect if both URL and connect are set
      if (enableConnection && currentCameraUrl && !currentUseHlsStreaming) {
        connectToWebSocket(currentCameraUrl);
      } else {
        showBlankScreen();
        updateConnectionStatus(false);
      }
      
      // Subscribe for property changes
      WebCC.onPropertyChanged.subscribe(setProperty);
    } else {
      console.error('WebCC: Connection failed');
      updateConnectionStatus(false);
    }
  },
  // contract (see also manifest.json)
  {
    methods: {},
    events: [],
    properties: {
      camera_stream_url: '',
      enable_connection: false,
      status_connected: false,
      status_fps: 0,
      status_bitrate_kbps: 0,
      show_version: false,
      camera_auth_token: '',
      use_control_stream: false,
      playback_start_time: '',
      playback_end_time: '',
      enable_playback: false,
      playback_speed: 1.0,
      enable_livestream: false,
      recording_reason: '',
      enable_recording: false,
      seek_to_time: '',
      status_timestamp: '',
      enable_debug: false,
      ptz_move: '',
      ptz_stop: false,
      ptz_goto_preset: '',
      ptz_set_preset: '',
      use_hls_streaming: false
    }
  },
  // placeholder to include additional Unified dependencies
  [],
  // connection timeout
  10000
);