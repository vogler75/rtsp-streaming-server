// Copyright 2022 Siemens AG. This file is subject to the terms and conditions of the MIT License. See LICENSE file in the top-level directory.
//
// SPDX-License-Identifier: MIT

////////////////////////////////////////////
// VideoPlayer internal properties

let websocket = null;
let videoElement = null;
let statusElement = null;
let isConnected = false;
let currentURL = '';
let shouldConnect = false;
let reconnectTimer = null;
let reconnectAttempts = 0;
let reconnectInterval = 3000;

////////////////////////////////////////////
// VideoPlayer functions

function initializeVideoPlayer() {
  videoElement = document.getElementById('videoPlayer');
  statusElement = document.getElementById('status');
  
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
  
  if (statusElement) {
    if (connected) {
      statusElement.textContent = 'Connected';
      statusElement.style.backgroundColor = 'rgba(0,128,0,0.7)';
      reconnectAttempts = 0;
    } else if (shouldConnect) {
      const reconnectText = reconnectAttempts > 0 ? ` (Retry ${reconnectAttempts})` : '';
      statusElement.textContent = 'Disconnected' + reconnectText;
      statusElement.style.backgroundColor = 'rgba(128,0,0,0.7)';
    } else {
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
    
    // For WSS with self-signed certificates, try different approaches
    if (url.startsWith('wss://')) {
      console.log('Attempting WSS connection (ignoring certificate errors where possible)');
      
      // Try to create WebSocket with additional error handling for certificate issues
      try {
        websocket = new WebSocket(url);
      } catch (certError) {
        console.warn('WSS connection failed, possibly due to certificate issues:', certError);
        
        // Fallback: try converting WSS to WS for testing
        const wsUrl = url.replace('wss://', 'ws://');
        console.log('Attempting fallback to WS:', wsUrl);
        websocket = new WebSocket(wsUrl);
      }
    } else {
      websocket = new WebSocket(url);
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
        }
        
        // Create blob URL and clean up previous one
        const previousSrc = imgElement.src;
        if (previousSrc && previousSrc.startsWith('blob:')) {
          URL.revokeObjectURL(previousSrc);
        }
        
        // Set new blob URL
        const blobUrl = URL.createObjectURL(event.data);
        imgElement.src = blobUrl;
        
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
      console.error('URL:', websocket.url);
      console.error('ReadyState:', websocket.readyState);
      console.error('Error event:', error);
      
      // Provide specific guidance for WSS certificate issues
      if (websocket.url.startsWith('wss://')) {
        console.error('====== WSS CONNECTION TROUBLESHOOTING ======');
        console.error('If this is a self-signed certificate error:');
        console.error('1. Open the server URL in a browser: ' + websocket.url.replace('wss://', 'https://'));
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
      console.log('URL was:', websocket.url);
      
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
  console.log('Property changed:', data.key, '=', data.value);
  
  switch (data.key) {
    case 'URL':
      if (data.value !== currentURL) {
        currentURL = data.value;
        reconnectAttempts = 0;
        if (shouldConnect) {
          connectToWebSocket(data.value);
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
      shouldConnect = WebCC.Properties.connect || false;
      
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
      connected: false
    }
  },
  // placeholder to include additional Unified dependencies
  [],
  // connection timeout
  10000
);