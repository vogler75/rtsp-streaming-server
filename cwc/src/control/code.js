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
let currentURL = '';
let currentToken = '';
let shouldConnect = false;
let reconnectTimer = null;
let reconnectAttempts = 0;
let reconnectInterval = 3000;

// FPS and bitrate tracking variables
let frameCount = 0;
let fpsCounter = 0;
let lastFpsTime = Date.now();
let bytesReceived = 0;
let lastBitrateTime = Date.now();

// Control mode variables
let isControlMode = false;
let controlWebSocket = null;
let currentPlayFrom = '';
let currentPlayTo = '';
let currentPlay = false;
let currentSpeed = 1.0;
let currentLive = false;
let currentRecordingReason = '';
let currentRecordingActive = false;
let currentGoto = '';
let debugEnabled = false;

// PTZ control variables
let currentPtzMove = '';
let currentPtzStop = false;
let currentPtzGotoPreset = '';
let currentPtzSetPreset = '';

// HLS mode variables
let currentPlayHls = false;

////////////////////////////////////////////
// Debug helper function
function debugLog(...args) {
  if (debugEnabled) {
    console.log('[DEBUG]', ...args);
  }
}

function debugError(...args) {
  if (debugEnabled) {
    console.error('[DEBUG]', ...args);
  }
}

function debugWarn(...args) {
  if (debugEnabled) {
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
  WebCC.Properties.connected = connected;
  
  // Reset statistics when disconnected
  if (!connected) {
    frameCount = 0;
    fpsCounter = 0;
    lastFpsTime = Date.now();
    bytesReceived = 0;
    lastBitrateTime = Date.now();
    WebCC.Properties.fps = 0;
    WebCC.Properties.kbs = 0;
  }
  
  if (statusElement) {
    if (connected) {
      // Hide status when connected
      statusElement.style.display = 'none';
      reconnectAttempts = 0;
    } else if (shouldConnect) {
      // Show status when disconnected but trying to connect
      statusElement.style.display = 'block';
      const reconnectText = reconnectAttempts > 0 ? ` (Retry ${reconnectAttempts})` : '';
      statusElement.textContent = 'Disconnected' + reconnectText;
      statusElement.style.backgroundColor = 'rgba(128,0,0,0.7)';
    } else {
      // Show status when stopped
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
  }
  
  // Only reconnect if shouldConnect is true AND not in HLS mode
  if (shouldConnect && currentURL && !currentPlayHls) {
    reconnectAttempts++;
    debugLog(`Scheduling reconnect attempt ${reconnectAttempts} in ${reconnectInterval}ms`);
    updateConnectionStatus(false);
    
    reconnectTimer = setTimeout(() => {
      if (shouldConnect && !currentPlayHls) { // Check again in case it changed during timeout
        debugLog(`Attempting to reconnect (${reconnectAttempts})`);
        connectToWebSocket(currentURL);
      }
    }, reconnectInterval);
  } else {
    debugLog('Not reconnecting: shouldConnect =', shouldConnect, 'currentURL =', currentURL, 'currentPlayHls =', currentPlayHls);
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
  if (currentPlayHls) {
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
    if (websocket) {
      websocket.close();
      websocket = null;
    }
    if (controlWebSocket) {
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
    if (shouldConnect && currentURL) {
      connectToWebSocket(currentURL);
    }
  }
}

function connectToWebSocket(url) {
  if (!url || url.trim() === '' || !shouldConnect || currentPlayHls) {
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
  if (isControlMode) {
    fullUrl = fullUrl + '/control';
    connectToControlWebSocket(fullUrl);
    return;
  } else {
    fullUrl = fullUrl + '/stream';
  }
  
  if (websocket) {
    websocket.close();
    websocket = null;
  }
  
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  
  try {
    debugLog('Attempting WebSocket connection...');
    debugLog('URL:', fullUrl);
    debugLog('Protocol:', fullUrl.startsWith('wss://') ? 'Secure WebSocket (WSS)' : 'WebSocket (WS)');
    
    // Add token to URL if provided
    let connectUrl = fullUrl;
    if (currentToken && currentToken.trim() !== '') {
      const separator = fullUrl.includes('?') ? '&' : '?';
      connectUrl = fullUrl + separator + 'token=' + encodeURIComponent(currentToken);
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
          WebCC.Properties.fps = fps;
          fpsCounter = 0;
          lastFpsTime = now;
        }
        
        // Bitrate calculation - update every second
        if (now - lastBitrateTime >= 1000) {
          const kbs = Math.round(bytesReceived / 1024); // KB/s
          WebCC.Properties.kbs = kbs;
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
        
      } else if (typeof event.data === 'string') {
        try {
          const data = JSON.parse(event.data);
          if (data.type === 'stream_url' && data.url) {
            // Handle video URL (show video element, hide img)
            videoElement.style.display = 'block';
            videoElement.src = data.url;
            
            const imgElement = document.getElementById('mjpegFrame');
            if (imgElement) {
              imgElement.style.display = 'none';
              // Clean up any remaining blob URL
              if (imgElement.src && imgElement.src.startsWith('blob:')) {
                URL.revokeObjectURL(imgElement.src);
              }
            }
          }
        } catch (e) {
          debugLog('Received text data:', event.data);
        }
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
        debugError('3. Or disable TLS in server config.toml and use ws:// instead');
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
    controlWebSocket.close();
    controlWebSocket = null;
  }
  
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }
  
  try {
    // URL already has /control appended by connectToWebSocket
    let controlUrl = url;
    
    // Only add token if provided - no other parameters
    if (currentToken && currentToken.trim() !== '') {
      const separator = controlUrl.includes('?') ? '&' : '?';
      controlUrl += separator + 'token=' + encodeURIComponent(currentToken);
      debugLog('Adding token to control URL');
    }
    
    debugLog('🔗 Control URL:', controlUrl);
    controlWebSocket = new WebSocket(controlUrl);
    
    controlWebSocket.onopen = function() {
      debugLog('✅ Control WebSocket connected successfully');
      updateConnectionStatus(true);
      
      // Auto-enable live mode when connection is established
      debugLog('📺 Auto-enabling live mode after connection established');
      
      // Update internal state and WebCC properties
      currentLive = true;
      currentPlay = false;
      WebCC.Properties.live = true;
      WebCC.Properties.play = false;
      
      // Send live command to start streaming
      sendControlCommand({ cmd: 'live' });
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
            WebCC.Properties.timestamp = timestampISO;
            
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
              WebCC.Properties.fps = fps;
              fpsCounter = 0;
              lastFpsTime = now;
            }
            
            // Bitrate calculation
            if (now - lastBitrateTime >= 1000) {
              const kbs = Math.round(bytesReceived / 1024);
              WebCC.Properties.kbs = kbs;
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
  if (!isControlMode || !currentURL) {
    debugLog('Recording control only available in control mode');
    return;
  }
  
  // Use HTTP API for recording control
  // Convert WebSocket URL to HTTP URL
  // currentURL is now just the base path like /cam1
  let baseUrl = currentURL.replace(/^ws(s?):\/\//, 'http$1://');
  
  // Build the full endpoint URL
  const endpoint = active ? '/control/recording/start' : '/control/recording/stop';
  const fullUrl = baseUrl + endpoint;
  
  const requestOptions = {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json'
    }
  };
  
  if (currentToken) {
    requestOptions.headers['Authorization'] = 'Bearer ' + currentToken;
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
  if (!currentURL) {
    debugLog('PTZ control requires URL to be set');
    return;
  }
  
  // Convert WebSocket URL to HTTP URL
  // currentURL is now just the base path like /cam1
  let baseUrl = currentURL.replace(/^ws(s?):\/\//, 'http$1://');
  
  // Build the full PTZ endpoint URL
  const fullUrl = baseUrl + '/control/ptz/' + endpoint;
  
  const requestOptions = {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json'
    }
  };
  
  if (currentToken) {
    requestOptions.headers['Authorization'] = 'Bearer ' + currentToken;
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
  
  // Clean up any existing HLS instance
  if (hls) {
    hls.destroy();
    hls = null;
  }
  
  if (Hls.isSupported()) {
    // Use HLS.js for browsers that don't support HLS natively
    hls = new Hls({
      debug: debugEnabled,
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
    case 'URL':
      if (data.value !== currentURL) {
        const oldURL = currentURL;
        currentURL = data.value;
        reconnectAttempts = 0;
        
        // If we're connected or trying to connect, disconnect first then reconnect
        if (shouldConnect) {
          // Disconnect from old URL if connected
          if (websocket) {
            debugLog('URL changed from', oldURL, 'to', currentURL, '- reconnecting...');
            websocket.close();
            websocket = null;
          }
          
          // Clear any pending reconnection timers
          if (reconnectTimer) {
            clearTimeout(reconnectTimer);
            reconnectTimer = null;
          }
          
          // Connect to new URL
          if (currentURL) {
            connectToWebSocket(currentURL);
          } else {
            // No URL provided, just update status
            updateConnectionStatus(false);
            showBlankScreen();
          }
        }
      }
      break;
    case 'connect':
      shouldConnect = data.value;
      if (shouldConnect && !currentPlayHls) {
        // Start connecting (only if not in HLS mode)
        reconnectAttempts = 0;
        if (currentURL) {
          connectToWebSocket(currentURL);
          
          // Live mode will be auto-enabled after connection is established
        }
      } else if (shouldConnect && currentPlayHls) {
        debugLog('Connect requested but in HLS mode - ignoring WebSocket connection');
      } else {
        // Disconnect and show blank screen
        if (websocket) {
          websocket.close();
          websocket = null;
        }
        if (controlWebSocket) {
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
    case 'version':
      if (versionElement) {
        versionElement.style.display = data.value ? 'block' : 'none';
      }
      break;
    case 'token':
      if (data.value !== currentToken) {
        currentToken = data.value;
        debugLog('Token updated');
        
        // If we're connected and token changed, reconnect with new token
        if (shouldConnect && currentURL) {
          if (websocket) {
            debugLog('Token changed - reconnecting with new authentication...');
            websocket.close();
            websocket = null;
          }
          
          // Clear any pending reconnection timers
          if (reconnectTimer) {
            clearTimeout(reconnectTimer);
            reconnectTimer = null;
          }
          
          // Reconnect with new token
          connectToWebSocket(currentURL);
        }
      }
      break;
    case 'control':
      isControlMode = data.value;
      debugLog('🔧 Control mode changed to:', isControlMode);
      
      // Reconnect if URL is available and we're supposed to be connected
      if (shouldConnect && currentURL) {
        debugLog('Reconnecting due to control mode change...');
        if (websocket) {
          websocket.close();
          websocket = null;
        }
        if (controlWebSocket) {
          controlWebSocket.close();
          controlWebSocket = null;
        }
        connectToWebSocket(currentURL);
      }
      break;
    case 'play_from':
      currentPlayFrom = data.value;
      debugLog('🔧 Play from timestamp changed to:', currentPlayFrom);
      break;
    case 'play_to':
      currentPlayTo = data.value;
      debugLog('🔧 Play to timestamp changed to:', currentPlayTo);
      break;
    case 'play':
      currentPlay = data.value;
      debugLog('🔧 Play control changed to:', currentPlay);
      
      if (currentPlayHls) {
        // HLS mode playback
        if (currentPlay && currentPlayFrom && currentURL) {
          const toTime = currentPlayTo || new Date().toISOString();
          const hlsUrl = buildHlsUrl(currentURL, currentPlayFrom, toTime);
          debugLog('🎬 Starting HLS playback from:', currentPlayFrom, 'to:', toTime);
          playHlsStream(hlsUrl);
        } else if (!currentPlay) {
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
          debugWarn('⚠️ HLS playback ignored - missing required parameters (play_from or URL)');
        }
      } else if (isControlMode && controlWebSocket) {
        // WebSocket control mode playback
        if (currentPlay) {
          // Start playback
          const command = {
            cmd: 'start',
            from: currentPlayFrom
          };
          if (currentPlayTo) {
            command.to = currentPlayTo;
          }
          debugLog('🎬 Triggering playback start with timestamps from:', currentPlayFrom, 'to:', currentPlayTo || '(end)');
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
    case 'speed':
      currentSpeed = data.value;
      debugLog('🔧 Playback speed changed to:', currentSpeed);
      
      if (isControlMode && controlWebSocket) {
        debugLog('⚡ Triggering speed change');
        sendControlCommand({
          cmd: 'speed',
          speed: currentSpeed
        });
      } else {
        debugWarn('⚠️ Speed command ignored - not in control mode or WebSocket not connected');
      }
      break;
    case 'live':
      currentLive = data.value;
      debugLog('🔧 Live stream control changed to:', currentLive);
      
      if (isControlMode && controlWebSocket) {
        if (currentLive) {
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
    case 'recording_active':
      currentRecordingActive = data.value;
      debugLog('🔧 Recording active changed to:', currentRecordingActive);
      
      if (isControlMode) {
        debugLog('🔴 Triggering recording control - active:', currentRecordingActive, 'reason:', currentRecordingReason);
        handleRecordingControl(currentRecordingActive, currentRecordingReason);
      } else {
        debugWarn('⚠️ Recording command ignored - not in control mode');
      }
      break;
    case 'goto':
      currentGoto = data.value;
      debugLog('🔧 Goto timestamp changed to:', currentGoto);
      
      if (isControlMode && controlWebSocket && currentGoto) {
        debugLog('🎯 Triggering goto command to timestamp:', currentGoto);
        sendControlCommand({
          cmd: 'goto',
          timestamp: currentGoto
        });
      } else if (!currentGoto) {
        debugWarn('⚠️ Goto command ignored - empty timestamp');
      } else {
        debugWarn('⚠️ Goto command ignored - not in control mode or WebSocket not connected');
      }
      break;
    case 'debug':
      debugEnabled = data.value;
      console.log('Debug logging:', debugEnabled ? 'enabled' : 'disabled');
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
    case 'play_hls':
      currentPlayHls = data.value;
      debugLog('🎬 HLS mode changed to:', currentPlayHls);
      switchPlayerMode();
      break;
  }
}

////////////////////////////////////////////
// Initialize the custom control
WebCC.start(
  function (result) {
    if (result) {
      console.log('WebCC: Connected successfully');
      initializeVideoPlayer();
      
      // Set initial properties
      currentURL = WebCC.Properties.URL || '';
      currentToken = WebCC.Properties.token || '';
      shouldConnect = WebCC.Properties.connect || false;
      isControlMode = WebCC.Properties.control || false;
      currentPlayFrom = WebCC.Properties.play_from || '';
      currentPlayTo = WebCC.Properties.play_to || '';
      currentPlay = WebCC.Properties.play || false;
      currentSpeed = WebCC.Properties.speed || 1.0;
      currentLive = WebCC.Properties.live || false;
      currentRecordingReason = WebCC.Properties.recording_reason || '';
      currentRecordingActive = WebCC.Properties.recording_active || false;
      currentGoto = WebCC.Properties.goto || '';
      debugEnabled = WebCC.Properties.debug || false;
      
      // Initialize PTZ properties
      currentPtzMove = WebCC.Properties.ptz_move || '';
      currentPtzStop = WebCC.Properties.ptz_stop || false;
      currentPtzGotoPreset = WebCC.Properties.ptz_goto_preset || '';
      currentPtzSetPreset = WebCC.Properties.ptz_set_preset || '';
      
      // Initialize HLS property
      currentPlayHls = WebCC.Properties.play_hls || false;
      
      // Check version property at startup
      if (versionElement && WebCC.Properties.version) {
        versionElement.style.display = 'block';
      }
      
      // Initialize player mode
      switchPlayerMode();
      
      // Connect if both URL and connect are set
      if (shouldConnect && currentURL && !currentPlayHls) {
        connectToWebSocket(currentURL);
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
      URL: '',
      connect: false,
      connected: false,
      fps: 0,
      kbs: 0,
      version: false,
      token: '',
      control: false,
      play_from: '',
      play_to: '',
      play: false,
      speed: 1.0,
      live: false,
      recording_reason: '',
      recording_active: false,
      goto: '',
      timestamp: '',
      debug: false,
      ptz_move: '',
      ptz_stop: false,
      ptz_goto_preset: '',
      ptz_set_preset: '',
      play_hls: false
    }
  },
  // placeholder to include additional Unified dependencies
  [],
  // connection timeout
  10000
);