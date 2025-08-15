// Copyright 2022 Siemens AG. This file is subject to the terms and conditions of the MIT License. See LICENSE file in the top-level directory.
//
// SPDX-License-Identifier: MIT

////////////////////////////////////////////
// VideoPlayer internal properties

let websocket = null;
let videoElement = null;
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

////////////////////////////////////////////
// VideoPlayer functions

function initializeVideoPlayer() {
  videoElement = document.getElementById('videoPlayer');
  statusElement = document.getElementById('status');
  versionElement = document.getElementById('versionDisplay');
  
  videoElement.onerror = function(e) {
    // Only log video errors if we're actually using the video element
    if (videoElement.style.display !== 'none') {
      console.error('Video error:', e);
      updateConnectionStatus(false);
    }
  };
  
  videoElement.onloadstart = function() {
    if (videoElement.style.display !== 'none') {
      console.log('Video loading started');
    }
  };
  
  videoElement.oncanplay = function() {
    if (videoElement.style.display !== 'none') {
      console.log('Video can start playing');
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
  
  console.log('Connection status:', connected);
}

function scheduleReconnect() {
  if (reconnectTimer) {
    clearTimeout(reconnectTimer);
  }
  
  // Only reconnect if shouldConnect is true (endless reconnection)
  if (shouldConnect && currentURL) {
    reconnectAttempts++;
    console.log(`Scheduling reconnect attempt ${reconnectAttempts} in ${reconnectInterval}ms`);
    updateConnectionStatus(false);
    
    reconnectTimer = setTimeout(() => {
      if (shouldConnect) { // Check again in case it changed during timeout
        console.log(`Attempting to reconnect (${reconnectAttempts})`);
        connectToWebSocket(currentURL);
      }
    }, reconnectInterval);
  } else {
    console.log('Not reconnecting: shouldConnect =', shouldConnect, 'currentURL =', currentURL);
  }
}

function showBlankScreen() {
  // Hide video/image elements
  videoElement.style.display = 'none';
  const imgElement = document.getElementById('mjpegFrame');
  if (imgElement) {
    imgElement.style.display = 'none';
    // Clean up any blob URL
    if (imgElement.src && imgElement.src.startsWith('blob:')) {
      URL.revokeObjectURL(imgElement.src);
    }
  }
}

function connectToWebSocket(url) {
  if (!url || url.trim() === '' || !shouldConnect) {
    console.log('No URL provided or connection disabled');
    updateConnectionStatus(false);
    return;
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
    console.log('Attempting WebSocket connection...');
    console.log('URL:', url);
    console.log('Protocol:', url.startsWith('wss://') ? 'Secure WebSocket (WSS)' : 'WebSocket (WS)');
    
    // Add token to URL if provided
    let connectUrl = url;
    if (currentToken && currentToken.trim() !== '') {
      const separator = url.includes('?') ? '&' : '?';
      connectUrl = url + separator + 'token=' + encodeURIComponent(currentToken);
      console.log('Using authentication token');
    }
    
    // For WSS with self-signed certificates, try different approaches
    if (connectUrl.startsWith('wss://')) {
      console.log('Attempting WSS connection (ignoring certificate errors where possible)');
      
      // Try to create WebSocket with additional error handling for certificate issues
      try {
        websocket = new WebSocket(connectUrl);
      } catch (certError) {
        console.warn('WSS connection failed, possibly due to certificate issues:', certError);
        
        // Fallback: try converting WSS to WS for testing
        const wsUrl = connectUrl.replace('wss://', 'ws://');
        console.log('Attempting fallback to WS:', wsUrl);
        websocket = new WebSocket(wsUrl);
      }
    } else {
      websocket = new WebSocket(connectUrl);
    }
    
    websocket.onopen = function() {
      console.log('WebSocket connected');
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
          console.log('Received text data:', event.data);
        }
      }
    };
    
    websocket.onerror = function(error) {
      console.error('WebSocket error occurred');
      console.error('URL:', currentURL);
      console.error('ReadyState:', websocket ? websocket.readyState : 'null');
      console.error('Error event:', error);
      
      // Provide specific guidance for WSS certificate issues
      if (currentURL.startsWith('wss://')) {
        console.error('====== WSS CONNECTION TROUBLESHOOTING ======');
        console.error('If this is a self-signed certificate error:');
        console.error('1. Open the server URL in a browser: ' + currentURL.replace('wss://', 'https://'));
        console.error('2. Accept the security warning to trust the certificate');
        console.error('3. Or disable TLS in server config.toml and use ws:// instead');
        console.error('4. Or add the certificate to the system trust store');
        console.error('============================================');
      }
      
      updateConnectionStatus(false);
      scheduleReconnect();
    };
    
    websocket.onclose = function(event) {
      console.log('WebSocket closed');
      console.log('Code:', event.code);
      console.log('Reason:', event.reason || 'No reason provided');
      console.log('Was clean:', event.wasClean);
      console.log('URL was:', currentURL);
      
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
      console.log('Close description:', closeDescription);
      
      updateConnectionStatus(false);
      websocket = null;
      
      if (event.code !== 1000) {
        scheduleReconnect();
      }
    };
    
  } catch (error) {
    console.error('Failed to create WebSocket:', error);
    updateConnectionStatus(false);
    scheduleReconnect();
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
            console.log('URL changed from', oldURL, 'to', currentURL, '- reconnecting...');
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
      if (shouldConnect) {
        // Start connecting
        reconnectAttempts = 0;
        if (currentURL) {
          connectToWebSocket(currentURL);
        }
      } else {
        // Disconnect and show blank screen
        if (websocket) {
          websocket.close();
          websocket = null;
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
        console.log('Token updated');
        
        // If we're connected and token changed, reconnect with new token
        if (shouldConnect && currentURL) {
          if (websocket) {
            console.log('Token changed - reconnecting with new authentication...');
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
  }
}

////////////////////////////////////////////
// Initialize the custom control
WebCC.start(
  function (result) {
    if (result) {
      console.log('WebCC connected successfully');
      initializeVideoPlayer();
      
      // Set initial properties
      currentURL = WebCC.Properties.URL || '';
      currentToken = WebCC.Properties.token || '';
      shouldConnect = WebCC.Properties.connect || false;
      
      // Check version property at startup
      if (versionElement && WebCC.Properties.version) {
        versionElement.style.display = 'block';
      }
      
      // Connect if both URL and connect are set
      if (shouldConnect && currentURL) {
        connectToWebSocket(currentURL);
      } else {
        showBlankScreen();
        updateConnectionStatus(false);
      }
      
      // Subscribe for property changes
      WebCC.onPropertyChanged.subscribe(setProperty);
    } else {
      console.log('WebCC connection failed');
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
      token: ''
    }
  },
  // placeholder to include additional Unified dependencies
  [],
  // connection timeout
  10000
);