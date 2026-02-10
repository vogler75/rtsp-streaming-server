let refreshTimer;
let currentCameras = [];
let isManualRefresh = false;
let isAdminMode = false;
let isRefreshing = false; // Prevent overlapping refresh calls

// Token Storage:
// - Admin token: Stored in localStorage as 'adminToken'  
// - Camera tokens: Stored in localStorage as 'cameraTokens' (JSON object)
// - Session timestamp: Stored as 'adminTokenTimestamp' for auto-logout
// - Not using cookies for compatibility and explicit control
// - Tokens are cleared when admin mode is disabled or session expires
let adminToken = localStorage.getItem('adminToken') || '';
// Get base path by removing '/dashboard' from the end of the pathname
const pathname = window.location.pathname;
const basePath = pathname.endsWith('/dashboard') 
    ? pathname.slice(0, -'/dashboard'.length) 
    : pathname.replace(/\/dashboard$/, '');

// Log base path for debugging proxy setups
console.log('Dashboard loaded from:', pathname);
console.log('Using base path for API calls:', basePath || '(root)');

// Session timeout: 4 hours (in milliseconds)
const SESSION_TIMEOUT = 4 * 60 * 60 * 1000;

// Check token expiration on load
function checkTokenExpiration() {
    const tokenTimestamp = localStorage.getItem('adminTokenTimestamp');
    if (adminToken && tokenTimestamp) {
        const now = Date.now();
        const tokenAge = now - parseInt(tokenTimestamp);
        if (tokenAge > SESSION_TIMEOUT) {
            // Token expired
            adminToken = '';
            localStorage.removeItem('adminToken');
            localStorage.removeItem('adminTokenTimestamp');
            showAlert('Admin session expired. Please authenticate again.', 'warning');
        }
    }
}

// Storage for camera tokens
const cameraTokens = {};

// Load camera tokens from localStorage
const storedCameraTokens = localStorage.getItem('cameraTokens');
if (storedCameraTokens) {
    try {
        const tokens = JSON.parse(storedCameraTokens);
        Object.assign(cameraTokens, tokens);
    } catch (e) {
        console.warn('Failed to parse stored camera tokens:', e);
    }
}

// Check token expiration first
checkTokenExpiration();

// Check if admin token is valid on load, or if no token is needed
checkAdminAccess();

async function checkAdminAccess() {
    try {
        // Test admin access by trying to get a camera config (using cam1 as test)
        const headers = {};
        if (adminToken) {
            headers['Authorization'] = `Bearer ${adminToken}`;
        }
        
        const response = await fetch(`${basePath}/api/admin/cameras/cam1`, { headers });
        
        if (response.ok) {
            enableAdminMode();
        } else if (response.status === 401) {
            adminToken = '';
            localStorage.removeItem('adminToken');
            localStorage.removeItem('adminTokenTimestamp');
        }
    } catch (error) {
        console.error('Error checking admin access:', error);
    }
}

function showAdminAuth() {
    document.getElementById('authModal').classList.add('show');
}

function closeAuthModal() {
    document.getElementById('authModal').classList.remove('show');
}

async function authenticateAdmin() {
    const token = document.getElementById('adminToken').value;
    
    if (!token) {
        showAlert('Please enter an admin token', 'error');
        return;
    }
    
    try {
        // Test admin access by trying to get server config (requires admin token)
        const response = await fetch(`${basePath}/api/admin/config`, {
            method: 'GET',
            headers: {
                'Authorization': `Bearer ${token}`
            }
        });
        
        if (response.ok) {
            adminToken = token;
            localStorage.setItem('adminToken', token);
            localStorage.setItem('adminTokenTimestamp', Date.now().toString());
            enableAdminMode();
            closeAuthModal();
            showAlert('Admin mode enabled', 'success');
        } else if (response.status === 401) {
            showAlert('Invalid admin token', 'error');
            document.getElementById('adminToken').value = '';
        } else {
            showAlert('Authentication failed: ' + response.statusText, 'error');
        }
    } catch (error) {
        showAlert('Authentication failed: ' + error.message, 'error');
    }
}

function enableAdminMode() {
    isAdminMode = true;
    document.getElementById('addCameraBtn').style.display = 'inline-block';
    document.getElementById('serverConfigBtn').style.display = 'inline-block';
    const adminBtn = document.querySelector('.admin-btn');
    adminBtn.textContent = '✓ Admin Mode (Click to Disable)';
    adminBtn.style.background = 'linear-gradient(135deg, #4caf50 0%, #8bc34a 100%)';
    adminBtn.onclick = disableAdminMode;
    // Refresh camera list to show admin buttons (edit/delete)
    refreshStatus(true);
}

function disableAdminMode() {
    isAdminMode = false;
    adminToken = '';
    localStorage.removeItem('adminToken');
    localStorage.removeItem('adminTokenTimestamp');
    
    // Hide admin buttons
    document.getElementById('addCameraBtn').style.display = 'none';
    document.getElementById('serverConfigBtn').style.display = 'none';
    
    // Reset admin button
    const adminBtn = document.querySelector('.admin-btn');
    adminBtn.textContent = '🔐 Admin Mode';
    adminBtn.style.background = 'linear-gradient(135deg, #667eea 0%, #764ba2 100%)';
    adminBtn.onclick = showAdminAuth;
    
    // Refresh camera list to hide admin buttons (edit/delete)
    refreshStatus(true);
    
    showAlert('Admin mode disabled', 'info');
}

function showAlert(message, type = 'info') {
    const alert = document.getElementById('alert');
    alert.className = `alert ${type} show`;
    alert.innerHTML = `<span>${message}</span><span class="alert-close" onclick="this.parentElement.classList.remove('show')">&times;</span>`;
}

function toggleSection(element) {
    element.classList.toggle('collapsed');
    element.nextElementSibling.classList.toggle('collapsed');
}

function toggleDatabaseOptions() {
    const databaseType = document.getElementById('config_recording_database_type').value;
    const databaseUrlGroup = document.getElementById('database_url_group');
    const databaseExamples = document.getElementById('database_examples');
    
    if (databaseType === 'postgresql') {
        databaseUrlGroup.style.display = 'block';
        databaseExamples.style.display = 'block';
    } else {
        databaseUrlGroup.style.display = 'none';
        databaseExamples.style.display = 'none';
    }
}

function showAddCamera() {
    if (!isAdminMode) {
        showAdminAuth();
        return;
    }
    
    document.getElementById('editingCameraId').value = '';
    document.getElementById('cameraForm').reset();
    document.getElementById('cameraId').disabled = false;
    document.querySelector('.modal-header h2').textContent = 'Add New Camera';
    document.getElementById('editModal').classList.add('active');
}

function showEditCamera(cameraId) {
    if (!isAdminMode) {
        showAdminAuth();
        return;
    }
    
    const headers = {};
    if (adminToken) {
        headers['Authorization'] = `Bearer ${adminToken}`;
    }
    
    fetch(`${basePath}/api/admin/cameras/${cameraId}`, { headers })
        .then(r => r.json())
        .then(data => {
            if (data.status === 'success') {
                populateForm({ camera_id: cameraId, config: data.data });
                document.getElementById('editingCameraId').value = cameraId;
                document.getElementById('cameraId').value = cameraId;
                document.getElementById('cameraId').disabled = true;
                document.querySelector('.modal-header h2').textContent = 'Edit Camera';
                document.getElementById('editModal').classList.add('active');
            } else {
                showAlert('Failed to load camera configuration', 'error');
            }
        })
        .catch(error => {
            console.error('Error loading camera config:', error);
            showAlert('Failed to load camera configuration', 'error');
        });
}

function populateForm(camera) {
    const config = camera.config;
    
    // Basic settings
    document.getElementById('enabled').value = config.enabled !== false ? 'true' : 'false';
    document.getElementById('path').value = config.path || '';
    document.getElementById('url').value = config.url || '';
    document.getElementById('transport').value = config.transport || 'tcp';
    document.getElementById('reconnect_interval').value = config.reconnect_interval || 5;
    document.getElementById('token').value = config.token || '';
    
    // Per-camera recording settings
    if (config.recording) {
        document.getElementById('session_segment_minutes').value = config.recording.session_segment_minutes || '';
        document.getElementById('frame_storage_enabled').value = (config.recording.frame_storage_enabled !== undefined && config.recording.frame_storage_enabled !== null) ? config.recording.frame_storage_enabled.toString() : '';
        document.getElementById('frame_storage_retention').value = config.recording.frame_storage_retention || '';
        document.getElementById('mp4_storage_type').value = config.recording.mp4_storage_type || '';
        document.getElementById('mp4_storage_retention').value = config.recording.mp4_storage_retention || '';
        document.getElementById('mp4_segment_minutes').value = config.recording.mp4_segment_minutes || '';
        // HLS settings
        document.getElementById('hls_storage_enabled').value = (config.recording.hls_storage_enabled !== undefined && config.recording.hls_storage_enabled !== null) ? config.recording.hls_storage_enabled.toString() : '';
        document.getElementById('hls_storage_retention').value = config.recording.hls_storage_retention || '';
        document.getElementById('hls_segment_seconds').value = config.recording.hls_segment_seconds || '';
        // Pre-recording buffer settings (memory-only, using new IDs)
        document.getElementById('pre_recording_enabled_camera').value = (config.recording.pre_recording_enabled !== undefined && config.recording.pre_recording_enabled !== null) ? config.recording.pre_recording_enabled.toString() : '';
        document.getElementById('pre_recording_buffer_minutes_camera').value = config.recording.pre_recording_buffer_minutes || '';
    } else {
        document.getElementById('session_segment_minutes').value = '';
        document.getElementById('frame_storage_enabled').value = '';
        document.getElementById('frame_storage_retention').value = '';
        document.getElementById('mp4_storage_type').value = '';
        document.getElementById('mp4_storage_retention').value = '';
        document.getElementById('mp4_segment_minutes').value = '';
        // HLS settings
        document.getElementById('hls_storage_enabled').value = '';
        document.getElementById('hls_storage_retention').value = '';
        document.getElementById('hls_segment_seconds').value = '';
        // Pre-recording buffer settings reset (memory-only, using new IDs)
        document.getElementById('pre_recording_enabled_camera').value = '';
        document.getElementById('pre_recording_buffer_minutes_camera').value = '';
    }
    
    // MQTT settings
    if (config.mqtt) {
        document.getElementById('mqtt_publish_interval').value = config.mqtt.publish_interval || 0;
        document.getElementById('mqtt_topic_name').value = config.mqtt.topic_name || '';
    }

    // PTZ settings
    if (config.ptz) {
        document.getElementById('ptz_enabled').value = (config.ptz.enabled || false).toString();
        document.getElementById('ptz_protocol').value = config.ptz.protocol || 'onvif';
        document.getElementById('ptz_onvif_url').value = config.ptz.onvif_url || '';
        document.getElementById('ptz_username').value = config.ptz.username || '';
        document.getElementById('ptz_password').value = config.ptz.password || '';
        document.getElementById('ptz_profile_token').value = config.ptz.profile_token || '';
    } else {
        document.getElementById('ptz_enabled').value = 'false';
        document.getElementById('ptz_protocol').value = 'onvif';
        document.getElementById('ptz_onvif_url').value = '';
        document.getElementById('ptz_username').value = '';
        document.getElementById('ptz_password').value = '';
        document.getElementById('ptz_profile_token').value = '';
    }
    togglePtzFields();
    
    // FFmpeg settings
    if (config.ffmpeg) {
        document.getElementById('ffmpeg_command').value = config.ffmpeg.command || '';
        document.getElementById('ffmpeg_quality').value = config.ffmpeg.quality || '';
        document.getElementById('ffmpeg_use_wallclock_as_timestamps').value = config.ffmpeg.use_wallclock_as_timestamps !== undefined && config.ffmpeg.use_wallclock_as_timestamps !== null ? config.ffmpeg.use_wallclock_as_timestamps.toString() : 'true';
        document.getElementById('ffmpeg_scale').value = config.ffmpeg.scale || '';
        document.getElementById('ffmpeg_output_framerate').value = config.ffmpeg.output_framerate || '';
        document.getElementById('ffmpeg_video_bitrate').value = config.ffmpeg.video_bitrate || '';
        document.getElementById('ffmpeg_rtbufsize').value = config.ffmpeg.rtbufsize || '';
        document.getElementById('ffmpeg_log_stderr').value = config.ffmpeg.log_stderr || '';
        document.getElementById('ffmpeg_fflags').value = config.ffmpeg.fflags || '';
        document.getElementById('ffmpeg_flags').value = config.ffmpeg.flags || '';
        document.getElementById('ffmpeg_avioflags').value = config.ffmpeg.avioflags || '';
        document.getElementById('ffmpeg_fps_mode').value = config.ffmpeg.fps_mode || '';
        document.getElementById('ffmpeg_data_timeout_secs').value = config.ffmpeg.data_timeout_secs || '';
    }
}

function closeEditModal() {
    document.getElementById('editModal').classList.remove('active');
}

let originalServerConfig = {};

async function showServerConfig() {
    if (!isAdminMode) {
        showAdminAuth();
        return;
    }
    
    try {
        const headers = {};
        if (adminToken) {
            headers['Authorization'] = `Bearer ${adminToken}`;
        }
        const response = await fetch(`${basePath}/api/admin/config`, { headers });
        
        if (response.ok) {
            const data = await response.json();
            if (data.status === 'success') {
                originalServerConfig = data.data;
                populateServerConfigForm(data.data);
                // Also populate the JSON editor with the current config
                const configJson = JSON.stringify(data.data, null, 2);
                document.getElementById('serverConfigEditor').value = configJson;
                document.getElementById('serverConfigModal').classList.add('active');
            } else {
                showAlert('Failed to load server configuration', 'error');
            }
        } else {
            showAlert('Unauthorized or failed to load configuration', 'error');
        }
    } catch (error) {
        showAlert(`Error loading configuration: ${error.message}`, 'error');
    }
}

function populateServerConfigForm(config) {
    // Server settings
    document.getElementById('config_server_host').value = config.server?.host || '';
    document.getElementById('config_server_port').value = config.server?.port || '';
    document.getElementById('config_server_cors_allow_origin').value = config.server?.cors_allow_origin || '';
    document.getElementById('config_server_admin_token').value = config.server?.admin_token || '';
    document.getElementById('config_server_cameras_directory').value = config.server?.cameras_directory || '';
    document.getElementById('config_server_mp4_export_path').value = config.server?.mp4_export_path || '';
    document.getElementById('config_server_mp4_export_max_jobs').value = config.server?.mp4_export_max_jobs || '';

    // TLS settings
    document.getElementById('config_server_tls_enabled').value = (config.server?.tls?.enabled || false).toString();
    document.getElementById('config_server_tls_cert_path').value = config.server?.tls?.cert_path || '';
    document.getElementById('config_server_tls_key_path').value = config.server?.tls?.key_path || '';
    
    // MQTT settings
    document.getElementById('config_mqtt_enabled').value = (config.mqtt?.enabled || false).toString();
    document.getElementById('config_mqtt_broker_url').value = config.mqtt?.broker_url || '';
    document.getElementById('config_mqtt_client_id').value = config.mqtt?.client_id || '';
    document.getElementById('config_mqtt_base_topic').value = config.mqtt?.base_topic || '';
    document.getElementById('config_mqtt_qos').value = (config.mqtt?.qos || 0).toString();
    document.getElementById('config_mqtt_retain').value = (config.mqtt?.retain || false).toString();
    document.getElementById('config_mqtt_keep_alive_secs').value = config.mqtt?.keep_alive_secs || '';
    document.getElementById('config_mqtt_publish_interval_secs').value = config.mqtt?.publish_interval_secs || '';
    document.getElementById('config_mqtt_publish_picture_arrival').value = (config.mqtt?.publish_picture_arrival !== undefined ? config.mqtt.publish_picture_arrival : true).toString();
    document.getElementById('config_mqtt_max_packet_size').value = config.mqtt?.max_packet_size || '';
    
    // Recording settings
    document.getElementById('config_recording_frame_storage_enabled').value = (config.recording?.frame_storage_enabled || false).toString();
    document.getElementById('config_recording_mp4_storage_type').value = config.recording?.mp4_storage_type || 'filesystem';
    document.getElementById('config_recording_database_type').value = config.recording?.database_type || 'sqlite';
    document.getElementById('config_recording_database_path').value = config.recording?.database_path || '';
    document.getElementById('config_recording_database_url').value = config.recording?.database_url || '';
    document.getElementById('config_recording_session_segment_minutes').value = config.recording?.session_segment_minutes || '';
    
    // Update database options display
    toggleDatabaseOptions();
    document.getElementById('config_recording_max_frame_size').value = config.recording?.max_frame_size || '';
    document.getElementById('config_recording_frame_storage_retention').value = config.recording?.frame_storage_retention || '';
    document.getElementById('config_recording_mp4_storage_path').value = config.recording?.mp4_storage_path || '';
    document.getElementById('config_recording_mp4_storage_retention').value = config.recording?.mp4_storage_retention || '';
    document.getElementById('config_recording_mp4_segment_minutes').value = config.recording?.mp4_segment_minutes || '';
    document.getElementById('config_recording_mp4_filename_include_reason').value = (config.recording?.mp4_filename_include_reason || false).toString();
    document.getElementById('config_recording_mp4_filename_use_local_time').value = (config.recording?.mp4_filename_use_local_time !== false).toString();
    document.getElementById('config_recording_cleanup_interval_minutes').value = config.recording?.cleanup_interval_minutes || '';
    // HLS settings
    document.getElementById('config_recording_hls_storage_enabled').value = (config.recording?.hls_storage_enabled || false).toString();
    document.getElementById('config_recording_hls_storage_retention').value = config.recording?.hls_storage_retention || '';
    document.getElementById('config_recording_hls_segment_seconds').value = config.recording?.hls_segment_seconds || '';
    
    // Pre-recording buffer settings (memory-only)
    document.getElementById('config_recording_pre_recording_enabled_new').value = (config.recording?.pre_recording_enabled || false).toString();
    document.getElementById('config_recording_pre_recording_buffer_minutes_new').value = config.recording?.pre_recording_buffer_minutes || '';
    document.getElementById('config_recording_pre_recording_cleanup_interval_seconds_new').value = config.recording?.pre_recording_cleanup_interval_seconds || '';
    
    // Transcoding settings
    document.getElementById('config_transcoding_output_format').value = config.transcoding?.output_format || 'mjpeg';
    document.getElementById('config_transcoding_capture_framerate').value = config.transcoding?.capture_framerate || '';
    document.getElementById('config_transcoding_output_framerate').value = config.transcoding?.output_framerate || '';
    document.getElementById('config_transcoding_channel_buffer_size').value = config.transcoding?.channel_buffer_size || '';
    document.getElementById('config_transcoding_debug_capture').value = (config.transcoding?.debug_capture || false).toString();
    document.getElementById('config_transcoding_debug_duplicate_frames').value = (config.transcoding?.debug_duplicate_frames || false).toString();
}

function collectServerConfigFromForm() {
    return {
        server: {
            host: document.getElementById('config_server_host').value || "0.0.0.0",
            port: parseInt(document.getElementById('config_server_port').value) || 8080,
            cors_allow_origin: document.getElementById('config_server_cors_allow_origin').value || "*",
            admin_token: document.getElementById('config_server_admin_token').value || "",
            cameras_directory: document.getElementById('config_server_cameras_directory').value || null,
            mp4_export_path: document.getElementById('config_server_mp4_export_path').value || "exports",
            mp4_export_max_jobs: parseInt(document.getElementById('config_server_mp4_export_max_jobs').value) || 100,
            tls: {
                enabled: document.getElementById('config_server_tls_enabled').value === 'true',
                cert_path: document.getElementById('config_server_tls_cert_path').value || "certs/server.crt",
                key_path: document.getElementById('config_server_tls_key_path').value || "certs/server.key"
            }
        },
        mqtt: {
            enabled: document.getElementById('config_mqtt_enabled').value === 'true',
            broker_url: document.getElementById('config_mqtt_broker_url').value || "",
            client_id: document.getElementById('config_mqtt_client_id').value || "",
            base_topic: document.getElementById('config_mqtt_base_topic').value || "",
            qos: parseInt(document.getElementById('config_mqtt_qos').value) || 0,
            retain: document.getElementById('config_mqtt_retain').value === 'true',
            keep_alive_secs: parseInt(document.getElementById('config_mqtt_keep_alive_secs').value) || 60,
            publish_interval_secs: parseInt(document.getElementById('config_mqtt_publish_interval_secs').value) || 1,
            publish_picture_arrival: document.getElementById('config_mqtt_publish_picture_arrival').value === 'true',
            max_packet_size: parseInt(document.getElementById('config_mqtt_max_packet_size').value) || 268435456
        },
        recording: {
            frame_storage_enabled: document.getElementById('config_recording_frame_storage_enabled').value === 'true',
            mp4_storage_type: document.getElementById('config_recording_mp4_storage_type').value || 'filesystem',
            mp4_storage_path: document.getElementById('config_recording_mp4_storage_path').value || null,
            database_type: document.getElementById('config_recording_database_type').value || 'sqlite',
            database_path: document.getElementById('config_recording_database_path').value || "recordings",
            database_url: document.getElementById('config_recording_database_url').value || null,
            session_segment_minutes: parseInt(document.getElementById('config_recording_session_segment_minutes').value) || 60,
            max_frame_size: parseInt(document.getElementById('config_recording_max_frame_size').value) || 10485760,
            frame_storage_retention: document.getElementById('config_recording_frame_storage_retention').value || "7d",
            mp4_storage_retention: document.getElementById('config_recording_mp4_storage_retention').value || "30d",
            mp4_segment_minutes: parseInt(document.getElementById('config_recording_mp4_segment_minutes').value) || 5,
            mp4_filename_include_reason: document.getElementById('config_recording_mp4_filename_include_reason').value === 'true',
            mp4_filename_use_local_time: document.getElementById('config_recording_mp4_filename_use_local_time').value === 'true',
            cleanup_interval_minutes: parseInt(document.getElementById('config_recording_cleanup_interval_minutes').value) || 60,
            hls_storage_enabled: document.getElementById('config_recording_hls_storage_enabled').value === 'true',
            hls_storage_retention: document.getElementById('config_recording_hls_storage_retention').value || "30d",
            hls_segment_seconds: parseInt(document.getElementById('config_recording_hls_segment_seconds').value) || 6,
            // Pre-recording buffer settings (memory-only)
            pre_recording_enabled: document.getElementById('config_recording_pre_recording_enabled_new').value === 'true',
            pre_recording_buffer_minutes: parseInt(document.getElementById('config_recording_pre_recording_buffer_minutes_new').value) || 1,
            pre_recording_cleanup_interval_seconds: parseInt(document.getElementById('config_recording_pre_recording_cleanup_interval_seconds_new').value) || 1
        },
        transcoding: {
            output_format: document.getElementById('config_transcoding_output_format').value || "mjpeg",
            capture_framerate: parseFloat(document.getElementById('config_transcoding_capture_framerate').value) || 0,
            output_framerate: parseFloat(document.getElementById('config_transcoding_output_framerate').value) || 0,
            channel_buffer_size: parseInt(document.getElementById('config_transcoding_channel_buffer_size').value) || 50,
            debug_capture: document.getElementById('config_transcoding_debug_capture').value === 'true',
            debug_duplicate_frames: document.getElementById('config_transcoding_debug_duplicate_frames').value === 'true'
        }
    };
}

function closeServerConfigModal() {
    document.getElementById('serverConfigModal').classList.remove('active');
}

function resetServerConfig() {
    populateServerConfigForm(originalServerConfig);
    showAlert('Configuration reset to original values', 'info');
}

function exportServerConfig() {
    const config = collectServerConfigFromForm();
    const configJson = JSON.stringify(config, null, 2);
    
    // Create download link
    const blob = new Blob([configJson], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'config.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    
    showAlert('Configuration exported as config.json', 'success');
}

async function saveServerConfig() {
    try {
        const config = collectServerConfigFromForm();
        
        const response = await fetch(`${basePath}/api/admin/config`, {
            method: 'PUT',
            headers: {
                'Content-Type': 'application/json',
                'Authorization': `Bearer ${adminToken}`
            },
            body: JSON.stringify(config)
        });
        
        if (response.ok) {
            const data = await response.json();
            if (data.status === 'success') {
                originalServerConfig = config; // Update original to new saved version
                const result = data.data;
                if (result.restart_required && result.camera_restart_recommended) {
                    const sections = (result.changed_sections || []).join('/');
                    showAlert(`Configuration saved. Server restart required. Cameras that inherit global ${sections} settings may also need restarting.`, 'warning');
                } else if (result.restart_required) {
                    showAlert('Configuration saved. Server restart required to apply changes.', 'warning');
                } else {
                    showAlert('Configuration saved (no changes detected).', 'success');
                }
                closeServerConfigModal();
            } else {
                showAlert(`Failed to save: ${data.message}`, 'error');
            }
        } else {
            const errorData = await response.text();
            showAlert(`Error saving configuration: ${errorData}`, 'error');
        }
    } catch (error) {
        showAlert(`Error saving configuration: ${error.message}`, 'error');
    }
}

// Toggle between form view and JSON view
function toggleConfigView() {
    const formView = document.getElementById('serverConfigForm');
    const jsonView = document.getElementById('serverConfigJson');
    const toggleBtn = document.getElementById('configViewToggle');
    
    if (formView.style.display === 'none') {
        // Switch to form view
        formView.style.display = 'block';
        jsonView.style.display = 'none';
        toggleBtn.textContent = '📝 Switch to JSON';
        
        // Sync JSON to form when switching to form view
        try {
            const jsonText = document.getElementById('serverConfigEditor').value;
            if (jsonText.trim()) {
                const config = JSON.parse(jsonText);
                populateServerConfigForm(config);
            }
        } catch (e) {
            showAlert('Invalid JSON detected - keeping form data unchanged', 'warning');
        }
    } else {
        // Switch to JSON view
        formView.style.display = 'none';
        jsonView.style.display = 'block';
        toggleBtn.textContent = '📄 Switch to Form';
        
        // Sync form to JSON when switching to JSON view
        const config = collectServerConfigFromForm();
        const configJson = JSON.stringify(config, null, 2);
        document.getElementById('serverConfigEditor').value = configJson;
    }
}

// Reset JSON editor to original configuration
function resetServerConfigJson() {
    const configJson = JSON.stringify(originalServerConfig, null, 2);
    document.getElementById('serverConfigEditor').value = configJson;
    showAlert('JSON editor reset to original values', 'info');
}

// Validate JSON in the editor
function validateServerConfig() {
    try {
        const jsonText = document.getElementById('serverConfigEditor').value;
        JSON.parse(jsonText);
        showAlert('JSON is valid!', 'success');
    } catch (e) {
        showAlert(`Invalid JSON: ${e.message}`, 'error');
    }
}

// Save configuration from JSON editor
async function saveServerConfigJson() {
    try {
        const jsonText = document.getElementById('serverConfigEditor').value;
        const config = JSON.parse(jsonText);
        
        const response = await fetch(`${basePath}/api/admin/config`, {
            method: 'PUT',
            headers: {
                'Content-Type': 'application/json',
                'Authorization': `Bearer ${adminToken}`
            },
            body: JSON.stringify(config)
        });
        
        if (response.ok) {
            const data = await response.json();
            if (data.status === 'success') {
                originalServerConfig = config; // Update original to new saved version
                const result = data.data;
                if (result.restart_required && result.camera_restart_recommended) {
                    const sections = (result.changed_sections || []).join('/');
                    showAlert(`Configuration saved. Server restart required. Cameras that inherit global ${sections} settings may also need restarting.`, 'warning');
                } else if (result.restart_required) {
                    showAlert('Configuration saved. Server restart required to apply changes.', 'warning');
                } else {
                    showAlert('Configuration saved (no changes detected).', 'success');
                }
                closeServerConfigModal();
            } else {
                showAlert(`Failed to save: ${data.message}`, 'error');
            }
        } else {
            const errorData = await response.text();
            showAlert(`Error saving configuration: ${errorData}`, 'error');
        }
    } catch (error) {
        if (error instanceof SyntaxError) {
            showAlert(`Invalid JSON: ${error.message}`, 'error');
        } else {
            showAlert(`Error saving configuration: ${error.message}`, 'error');
        }
    }
}

async function deleteCamera(cameraId) {
    if (!isAdminMode) {
        showAdminAuth();
        return;
    }
    
    if (!confirm(`Are you sure you want to delete camera ${cameraId}?`)) {
        return;
    }
    
    try {
        const response = await fetch(`${basePath}/api/admin/cameras/${cameraId}`, {
            method: 'DELETE',
            headers: {
                'Authorization': `Bearer ${adminToken}`
            }
        });
        
        const data = await response.json();
        
        if (data.status === 'success') {
            showAlert(`Camera ${cameraId} deleted successfully`, 'success');
            refreshStatus();
        } else {
            showAlert(data.error || 'Failed to delete camera', 'error');
        }
    } catch (error) {
        showAlert('Error deleting camera', 'error');
    }
}

document.getElementById('cameraForm').addEventListener('submit', async (e) => {
    e.preventDefault();
    
    const formData = new FormData(e.target);
    const isEditing = document.getElementById('editingCameraId').value;
    const cameraId = isEditing || formData.get('cameraId');
    
    // Build camera config
    const config = {
        enabled: formData.get('enabled') === 'true',
        path: formData.get('path'),
        url: formData.get('url'),
        transport: formData.get('transport'),
        reconnect_interval: parseInt(formData.get('reconnect_interval')),
        token: formData.get('token') || null
    };
    
    // Add per-camera recording settings if configured
    const sessionSegmentMinutes = formData.get('session_segment_minutes');
    const frameStorageEnabled = formData.get('frame_storage_enabled');
    const frameStorageRetention = formData.get('frame_storage_retention');
    const videoStorageType = formData.get('mp4_storage_type');
    const videoStorageRetention = formData.get('mp4_storage_retention');
    const videoSegmentMinutes = formData.get('mp4_segment_minutes');
    // HLS settings
    const hlsStorageEnabled = formData.get('hls_storage_enabled');
    const hlsStorageRetention = formData.get('hls_storage_retention');
    const hlsSegmentSeconds = formData.get('hls_segment_seconds');
    // Pre-recording buffer settings
    const preRecordingEnabled = formData.get('pre_recording_enabled_camera');
    const preRecordingBufferMinutes = formData.get('pre_recording_buffer_minutes_camera');
    
    // Only add recording section if at least one setting is configured
    if (sessionSegmentMinutes || 
        (frameStorageEnabled !== '' && frameStorageEnabled !== null) ||
        frameStorageRetention || videoStorageType || videoStorageRetention || videoSegmentMinutes ||
        (hlsStorageEnabled !== '' && hlsStorageEnabled !== null) || hlsStorageRetention || hlsSegmentSeconds ||
        (preRecordingEnabled !== '' && preRecordingEnabled !== null) || preRecordingBufferMinutes) {
        config.recording = {};
        
        if (sessionSegmentMinutes) {
            config.recording.session_segment_minutes = parseInt(sessionSegmentMinutes);
        }
        if (frameStorageEnabled !== '' && frameStorageEnabled !== null) {
            config.recording.frame_storage_enabled = frameStorageEnabled === 'true';
        }
        if (frameStorageRetention) {
            config.recording.frame_storage_retention = frameStorageRetention;
        }
        if (videoStorageType !== '' && videoStorageType !== null) {
            config.recording.mp4_storage_type = videoStorageType;
        }
        if (videoStorageRetention !== '' && videoStorageRetention !== null) {
            config.recording.mp4_storage_retention = videoStorageRetention;
        }
        if (videoSegmentMinutes) {
            config.recording.mp4_segment_minutes = parseInt(videoSegmentMinutes);
        }
        // HLS settings
        if (hlsStorageEnabled !== '' && hlsStorageEnabled !== null) {
            config.recording.hls_storage_enabled = hlsStorageEnabled === 'true';
        }
        if (hlsStorageRetention) {
            config.recording.hls_storage_retention = hlsStorageRetention;
        }
        if (hlsSegmentSeconds) {
            config.recording.hls_segment_seconds = parseInt(hlsSegmentSeconds);
        }
        
        // Pre-recording buffer settings (memory-only, using new field names)
        if (preRecordingEnabled !== '' && preRecordingEnabled !== null) {
            config.recording.pre_recording_enabled = preRecordingEnabled === 'true';
        }
        if (preRecordingBufferMinutes) {
            config.recording.pre_recording_buffer_minutes = parseInt(preRecordingBufferMinutes);
        }
    }
    
    // Add MQTT config if configured
    const mqttInterval = formData.get('mqtt_publish_interval');
    const mqttTopic = formData.get('mqtt_topic_name');
    if (mqttInterval || mqttTopic) {
        config.mqtt = {
            publish_interval: parseInt(mqttInterval) || 0,
            topic_name: mqttTopic || null
        };
    }
    
    // Add FFmpeg config
    const ffmpegConfig = {};
    const ffmpegFields = [
        'command', 'quality', 'use_wallclock_as_timestamps', 'scale', 'output_framerate', 'video_bitrate',
        'rtbufsize', 'log_stderr', 'fflags', 'flags', 'avioflags', 'fps_mode', 'data_timeout_secs'
    ];
    
    ffmpegFields.forEach(field => {
        const value = formData.get(`ffmpeg_${field}`);
        if (value) {
            if (field === 'quality' || field === 'output_framerate' || field === 'rtbufsize' || field === 'data_timeout_secs') {
                ffmpegConfig[field] = parseInt(value);
            } else if (field === 'use_wallclock_as_timestamps') {
                ffmpegConfig[field] = value === 'true';
            } else {
                ffmpegConfig[field] = value;
            }
        }
    });
    
    if (Object.keys(ffmpegConfig).length > 0) {
        config.ffmpeg = ffmpegConfig;
    }

    // Add PTZ config
    const ptzEnabled = formData.get('ptz_enabled') === 'true';
    const ptzProtocol = formData.get('ptz_protocol') || 'onvif';
    if (ptzEnabled) {
        config.ptz = {
            enabled: true,
            protocol: ptzProtocol,
            onvif_url: formData.get('ptz_onvif_url') || null,
            username: formData.get('ptz_username') || null,
            password: formData.get('ptz_password') || null,
            profile_token: formData.get('ptz_profile_token') || null
        };
    } else {
        // Explicitly disable if user selects No
        config.ptz = { enabled: false, protocol: ptzProtocol };
    }
    
    try {
        const url = isEditing ? 
            `${basePath}/api/admin/cameras/${cameraId}` : 
            `${basePath}/api/admin/cameras`;
            
        const method = isEditing ? 'PUT' : 'POST';
        const body = isEditing ? config : { camera_id: cameraId, config };
        
        const response = await fetch(url, {
            method,
            headers: {
                'Content-Type': 'application/json',
                'Authorization': `Bearer ${adminToken}`
            },
            body: JSON.stringify(body)
        });
        
        const data = await response.json();
        
        if (data.status === 'success') {
            showAlert(`Camera ${cameraId} ${isEditing ? 'updated' : 'created'} successfully. Changes applied immediately.`, 'success');
            closeEditModal();
            refreshStatus();
        } else {
            showAlert(data.error || 'Failed to save camera', 'error');
        }
    } catch (error) {
        showAlert('Error saving camera', 'error');
    }
});

function togglePtzFields() {
    const enabled = document.getElementById('ptz_enabled').value === 'true';
    const ids = ['ptz_protocol', 'ptz_onvif_url', 'ptz_username', 'ptz_password', 'ptz_profile_token'];
    ids.forEach(id => {
        const el = document.getElementById(id);
        if (el) el.disabled = !enabled;
    });
}

function hasCameraListChanged(newCameras) {
    if (currentCameras.length !== newCameras.length) {
        return true;
    }
    
    // Check if same cameras exist (by ID)
    const currentIds = new Set(currentCameras.map(c => c.id));
    const newIds = new Set(newCameras.map(c => c.id));
    
    for (const id of newIds) {
        if (!currentIds.has(id)) {
            return true;
        }
    }
    
    return false;
}

async function refreshStatus(forceFullRebuild = false) {
    // Prevent overlapping refresh calls
    if (isRefreshing) {
        console.log('Refresh already in progress, skipping...');
        return;
    }
    
    isRefreshing = true;
    
    try {
        // Load server status and cameras in parallel
        const [statusResponse, camerasResponse] = await Promise.all([
            fetch(`${basePath}/api/status`),
            fetch(`${basePath}/api/cameras`)
        ]);
        
        const statusData = await statusResponse.json();
        const camerasData = await camerasResponse.json();
        
        if (statusData.status === 'success' && camerasData.status === 'success') {
            updateServerStatus(statusData.data, camerasData.data.cameras);
            
            const newCameras = camerasData.data.cameras;
            const needsFullRebuild = forceFullRebuild || hasCameraListChanged(newCameras);
            
            if (needsFullRebuild) {
                // console.log('Performing full camera grid rebuild');
                await updateCameraGrid(newCameras);
            } else {
                // console.log('Performing differential camera update');
                await updateExistingCameras(newCameras);
            }
        } else {
            throw new Error('Failed to load server data');
        }
    } catch (error) {
        console.error('Error fetching status:', error);
        document.getElementById('serverStatus').textContent = 'Offline';
        document.getElementById('serverStatus').style.color = '#f44336';
    } finally {
        // Always reset the refresh flag
        isRefreshing = false;
    }
}

function updateServerStatus(statusData, cameras) {
    document.getElementById('serverStatus').textContent = 'Online';
    document.getElementById('serverStatus').style.color = '#4caf50';
    document.getElementById('serverUptime').textContent = formatUptime(statusData.uptime_secs);
    document.getElementById('activeCameras').textContent = statusData.total_cameras;
    document.getElementById('totalConnections').textContent = statusData.total_clients;
    
    // Update version display if available
    if (statusData.version) {
        document.getElementById('versionDisplay').textContent = `Version: ${statusData.version}`;
    }
    
    // Calculate recording cameras from camera data
    const recordingCameras = cameras.filter(cam => cam.ffmpeg_running).length;
    document.getElementById('recordingStatus').textContent = 
        recordingCameras > 0 ? `${recordingCameras} Active` : 'Inactive';
    
    // MQTT status - assume connected if server is running (could be enhanced later)
    document.getElementById('mqttStatus').textContent = 'Connected';
}

async function updateExistingCameras(cameras) {
    currentCameras = cameras; // Update stored cameras
    
    // Update each existing camera tile with new data
    for (const camera of cameras) {
        await updateCameraTile(camera);
    }
    
    // Update master stream checkbox
    setTimeout(updateMasterStreamCheckbox, 100);
}

async function updateCameraTile(camera) {
    const requiresToken = camera.token_required === true;
    const isOnline = camera.connected || camera.ffmpeg_running;
    const isEnabled = camera.enabled !== false; // Default to true if not specified

    // Use specific IDs to update elements
    const statusElement = document.getElementById(`status-${camera.id}`);
    if (statusElement) {
        statusElement.textContent = isOnline ? 'Online' : 'Offline';
    }

    const indicatorElement = document.getElementById(`indicator-${camera.id}`);
    if (indicatorElement) {
        indicatorElement.className = `status-indicator ${isOnline ? '' : 'offline'}`;
    }

    const fpsElement = document.getElementById(`fps-${camera.id}`);
    if (fpsElement) {
        fpsElement.textContent = camera.capture_fps.toFixed(1);
    }

    const clientsElement = document.getElementById(`clients-${camera.id}`);
    if (clientsElement) {
        clientsElement.textContent = camera.clients_connected;
    }

    const frameElement = document.getElementById(`frame-${camera.id}`);
    if (frameElement) {
        frameElement.textContent = camera.last_frame_time ? new Date(camera.last_frame_time).toLocaleTimeString() : 'Never';
    }

    const preBufferElement = document.getElementById(`pre-buffer-${camera.id}`);
    if (preBufferElement) {
        preBufferElement.textContent = `${camera.pre_recording_buffer_frames} frames (${camera.pre_recording_buffer_size_kb} KB)`;
    }

    const mp4BufferElement = document.getElementById(`mp4-buffer-${camera.id}`);
    if (mp4BufferElement) {
        mp4BufferElement.textContent = `${camera.mp4_buffered_frames} frames (${camera.mp4_buffered_size_kb} KB)`;
    }

    // Check if embedded stream needs to be stopped due to camera going offline
    const checkbox = document.getElementById(`stream-checkbox-${camera.id}`);
    if (checkbox && checkbox.checked && !isOnline) {
        showAlert(`Camera ${camera.id} went offline - stopping embedded stream`, 'warning');
        checkbox.checked = false;
        toggleEmbeddedStream(camera.id, camera.path, requiresToken);
    }

    // Update admin buttons visibility based on admin mode
    const cameraActionsDiv = document.getElementById(`actions-${camera.id}`);
    if (cameraActionsDiv) {
        const editBtn = cameraActionsDiv.querySelector('button[onclick*="showEditCamera"]');
        const deleteBtn = cameraActionsDiv.querySelector('.delete-btn');

        if (editBtn) editBtn.style.display = isAdminMode ? 'inline-block' : 'none';
        if (deleteBtn) deleteBtn.style.display = isAdminMode ? 'inline-block' : 'none';
    }

    // Update recording status only for enabled cameras
    if (isEnabled) {
        updateRecordingStatus(camera.id, camera.path, requiresToken);
    }
}

async function updateCameraGrid(cameras) {
    currentCameras = cameras; // Store cameras globally for recording status
    
    // Save current scroll position
    const scrollY = window.scrollY;
    
    const grid = document.getElementById('camerasGrid');
    grid.innerHTML = '';
    
    // Create all camera tiles in parallel with recording data
    const tilePromises = cameras.map(camera => createCameraTileWithRecording(camera));
    const tiles = await Promise.all(tilePromises);
    
    tiles.forEach(tile => {
        grid.appendChild(tile);
    });
    
    // Restore scroll position after DOM update
    setTimeout(() => {
        window.scrollTo(0, scrollY);
        // Update master stream checkbox after tiles are created
        updateMasterStreamCheckbox();
    }, 0);
}

async function createCameraTileWithRecording(camera) {
    const requiresToken = camera.token_required === true;
    const isEnabled = camera.enabled !== false; // Default to true if not specified

    // Fetch recording data in parallel
    let recordingStatus = 'Unknown';
    let recordingActive = false;
    let dbSize = 'Unknown';
    let recordingBtnText = '🔴 Start Recording';
    let recordingBtnColor = '#27ae60';
    let recordingAvailable = false;

    // Skip recording status checks for disabled cameras
    if (isEnabled) {
        try {
            const headers = { 'Content-Type': 'application/json' };

            if (requiresToken) {
                // Check if we have a saved token for this camera
                const savedToken = cameraTokens[camera.id];
                if (savedToken) {
                    headers['Authorization'] = `Bearer ${savedToken}`;
                }
            }

            if (adminToken) {
                headers['Authorization'] = `Bearer ${adminToken}`;
            }

            const [statusResponse, sizeResponse] = await Promise.all([
                fetch(`${basePath}${camera.path}/control/recording/active`, { headers }),
                fetch(`${basePath}${camera.path}/control/recording/size`, { headers })
            ]);

            if (statusResponse.ok) {
                recordingAvailable = true;
                const statusData = await statusResponse.json();
                recordingActive = statusData.status === 'success' && statusData.data && statusData.data.active;
                recordingStatus = recordingActive ? 'Active' : 'Stopped';
                recordingBtnText = recordingActive ? '⏹️ Stop Recording' : '🔴 Start Recording';
                recordingBtnColor = recordingActive ? '#e74c3c' : '#27ae60';
            }

            if (sizeResponse.ok) {
                recordingAvailable = true;
                const sizeData = await sizeResponse.json();
                if (sizeData.status === 'success' && sizeData.data) {
                    dbSize = formatFileSize(sizeData.data.size_bytes);
                }
            }
        } catch (error) {
            // Use default values for errors
        }
    }

    return createCameraTile(camera, recordingStatus, recordingActive, dbSize, recordingBtnText, recordingBtnColor, recordingAvailable);
}

function createCameraTile(camera, recordingStatus = 'Loading...', recordingActive = false, dbSize = 'Loading...', recordingBtnText = '🔴 Recording', recordingBtnColor = '#27ae60', recordingAvailable = false) {
    const tile = document.createElement('div');
    tile.className = 'camera-tile';
    
    const isOnline = camera.connected || camera.ffmpeg_running;
    const streamId = `stream-${camera.id}`;
    const requiresToken = camera.token_required === true;
    
    const adminButtons = isAdminMode ? `
        <button onclick="showEditCamera('${camera.id}')">✏️ Edit</button>
        <button class="delete-btn" onclick="deleteCamera('${camera.id}')">🗑️ Delete</button>
    ` : '';
    
    // Token input section for cameras that require tokens
    const tokenSection = requiresToken ? `
        <div class="camera-token">
            <div class="info-row">
                <span class="info-label">🔐 Token:</span>
                <input type="password" id="token-${camera.id}" placeholder="Enter token" 
                       style="width: 120px; padding: 3px 6px; border: 1px solid #ddd; border-radius: 3px; font-size: 11px;">
            </div>
        </div>
    ` : '';
    
    tile.innerHTML = `
        <div class="camera-header">
            <span class="camera-name">${camera.id}</span>
            <div class="camera-status">
                <span id="indicator-${camera.id}" class="status-indicator ${isOnline ? '' : 'offline'}"></span>
                <span id="status-${camera.id}">${isOnline ? 'Online' : 'Offline'}</span>
            </div>
        </div>
        <div class="camera-preview">
            <div class="preview-controls">
                <label style="display: flex; align-items: center; gap: 5px; font-size: 12px; color: white; cursor: pointer;">
                    <input type="checkbox" id="stream-checkbox-${camera.id}" onchange="toggleEmbeddedStream('${camera.id}', '${camera.path}', ${requiresToken})" style="margin: 0;">
                    📺 Live Stream
                </label>
            </div>
            <div id="stream-container-${camera.id}" class="stream-container" style="display: none;">
                <!-- Embedded stream iframe will be inserted here -->
            </div>
            <div id="no-preview-${camera.id}" class="no-preview" onclick="toggleStreamPreview('${camera.id}', '${camera.path}')">📷 ${camera.path}</div>
        </div>
        <div class="camera-info">
            <div class="info-row">
                <span class="info-label">FPS:</span>
                <span id="fps-${camera.id}">${camera.capture_fps.toFixed(1)}</span>
            </div>
            <div class="info-row">
                <span class="info-label">Clients:</span>
                <span id="clients-${camera.id}">${camera.clients_connected}</span>
            </div>
            <div class="info-row">
                <span class="info-label">Last Frame:</span>
                <span id="frame-${camera.id}">${camera.last_frame_time ? new Date(camera.last_frame_time).toLocaleTimeString() : 'Never'}</span>
            </div>
            <div class="info-row">
                <span class="info-label">Pre-Buffer:</span>
                <span id="pre-buffer-${camera.id}">${camera.pre_recording_buffer_frames} frames (${camera.pre_recording_buffer_size_kb} KB)</span>
            </div>
            <div class="info-row">
                <span class="info-label">MP4 Buffer:</span>
                <span id="mp4-buffer-${camera.id}">${camera.mp4_buffered_frames} frames (${camera.mp4_buffered_size_kb} KB)</span>
            </div>
            ${recordingAvailable ? `
            <div class="info-row">
                <span class="info-label">Recording:</span>
                <span id="recording-status-${camera.id}" class="recording-status-badge ${recordingActive ? 'active' : 'stopped'}">${recordingStatus}</span>
                <button id="recording-btn-${camera.id}" onclick="toggleRecording('${camera.id}', '${camera.path}', ${requiresToken})" style="background: ${recordingBtnColor}; margin-left: 10px; padding: 2px 8px; font-size: 12px; border: none; border-radius: 3px; color: white; cursor: pointer;">${recordingBtnText}</button>
            </div>
            <div class="info-row">
                <span class="info-label">DB Size:</span>
                <span id="db-size-${camera.id}">${dbSize}</span>
            </div>` : ''}
            ${tokenSection}
        </div>
        <div id="actions-${camera.id}" class="camera-actions">
            <button onclick="openCameraStream('${camera.id}', '${camera.path}', ${requiresToken})">🔗 Stream</button>
            <button onclick="openCameraControl('${camera.id}', '${camera.path}', ${requiresToken})">🎮 Control</button>
            <button onclick="showEditCamera('${camera.id}')" style="display: ${isAdminMode ? 'inline-block' : 'none'};">✏️ Edit</button>
            <button class="delete-btn" onclick="deleteCamera('${camera.id}')" style="display: ${isAdminMode ? 'inline-block' : 'none'};">🗑️ Delete</button>
        </div>
    `;
    
    // Restore saved token if available
    if (requiresToken && cameraTokens[camera.id]) {
        setTimeout(() => {
            const tokenInput = document.getElementById(`token-${camera.id}`);
            if (tokenInput) {
                tokenInput.value = cameraTokens[camera.id];
            }
        }, 0);
    }
    
    return tile;
}

function openCameraStream(cameraId, cameraPath, requiresToken) {
    let url = `${basePath}${cameraPath}/stream`;
    
    if (requiresToken) {
        const tokenInput = document.getElementById(`token-${cameraId}`);
        if (tokenInput && tokenInput.value.trim()) {
            const token = tokenInput.value.trim();
            // Save token for this camera
            cameraTokens[cameraId] = token;
            localStorage.setItem('cameraTokens', JSON.stringify(cameraTokens));
            url += `?token=${encodeURIComponent(token)}`;
        } else {
            showAlert('Please enter a token for this camera', 'error');
            return;
        }
    }
    
    window.open(url, '_blank');
}

function openCameraControl(cameraId, cameraPath, requiresToken) {
    let url = `${basePath}${cameraPath}/control`;
    
    if (requiresToken) {
        const tokenInput = document.getElementById(`token-${cameraId}`);
        if (tokenInput && tokenInput.value.trim()) {
            const token = tokenInput.value.trim();
            // Save token for this camera
            cameraTokens[cameraId] = token;
            localStorage.setItem('cameraTokens', JSON.stringify(cameraTokens));
            url += `?token=${encodeURIComponent(token)}`;
        } else {
            showAlert('Please enter a token for this camera', 'error');
            return;
        }
    }
    
    window.open(url, '_blank');
}

function toggleAllStreams() {
    const masterCheckbox = document.getElementById('allStreamsToggle');
    const isEnabled = masterCheckbox.checked;
    
    if (!currentCameras || currentCameras.length === 0) {
        showAlert('No cameras available', 'warning');
        masterCheckbox.checked = false;
        return;
    }
    
    let missingTokens = [];
    
    // Check for cameras that require tokens but don't have them
    if (isEnabled) {
        for (const camera of currentCameras) {
            const requiresToken = camera.token_required === true;
            if (requiresToken) {
                const tokenInput = document.getElementById(`token-${camera.id}`);
                if (!tokenInput || !tokenInput.value.trim()) {
                    missingTokens.push(camera.id);
                }
            }
        }
        
        if (missingTokens.length > 0) {
            showAlert(`Please enter tokens for cameras: ${missingTokens.join(', ')}`, 'error');
            masterCheckbox.checked = false;
            return;
        }
    }
    
    // Toggle all individual camera stream checkboxes
    for (const camera of currentCameras) {
        const checkbox = document.getElementById(`stream-checkbox-${camera.id}`);
        if (checkbox && checkbox.checked !== isEnabled) {
            checkbox.checked = isEnabled;
            const requiresToken = camera.token_required === true;
            toggleEmbeddedStream(camera.id, camera.path, requiresToken);
        }
    }
    
    const action = isEnabled ? 'enabled' : 'disabled';
    showAlert(`All camera streams ${action}`, 'success');
}

function updateMasterStreamCheckbox() {
    if (!currentCameras || currentCameras.length === 0) return;
    
    const masterCheckbox = document.getElementById('allStreamsToggle');
    const individualCheckboxes = currentCameras.map(camera => 
        document.getElementById(`stream-checkbox-${camera.id}`)
    ).filter(cb => cb !== null);
    
    if (individualCheckboxes.length === 0) return;
    
    const allChecked = individualCheckboxes.every(cb => cb.checked);
    const noneChecked = individualCheckboxes.every(cb => !cb.checked);
    
    if (allChecked) {
        masterCheckbox.checked = true;
        masterCheckbox.indeterminate = false;
    } else if (noneChecked) {
        masterCheckbox.checked = false;
        masterCheckbox.indeterminate = false;
    } else {
        masterCheckbox.checked = false;
        masterCheckbox.indeterminate = true;
    }
}

function toggleEmbeddedStream(cameraId, cameraPath, requiresToken) {
    const checkbox = document.getElementById(`stream-checkbox-${cameraId}`);
    const streamContainer = document.getElementById(`stream-container-${cameraId}`);
    const noPreview = document.getElementById(`no-preview-${cameraId}`);
    
    if (checkbox.checked) {
        // Start embedded stream
        let streamUrl = `${basePath}${cameraPath}/stream`;
        
        if (requiresToken) {
            const tokenInput = document.getElementById(`token-${cameraId}`);
            if (tokenInput && tokenInput.value.trim()) {
                const token = tokenInput.value.trim();
                streamUrl += `?token=${encodeURIComponent(token)}`;
            } else {
                showAlert('Please enter a token for this camera first', 'error');
                checkbox.checked = false;
                return;
            }
        }
        
        // Create iframe for embedded stream
        const iframe = document.createElement('iframe');
        iframe.src = streamUrl;
        iframe.style.width = '100%';
        iframe.style.height = '100%';
        iframe.style.border = 'none';
        iframe.allowFullscreen = true;
        
        streamContainer.innerHTML = '';
        streamContainer.appendChild(iframe);
        streamContainer.style.display = 'block';
        noPreview.style.display = 'none';
        
        console.log(`Started embedded stream for ${cameraId}: ${streamUrl}`);
    } else {
        // Stop embedded stream
        // First, properly dispose of the iframe to stop its JavaScript execution
        const existingIframe = streamContainer.querySelector('iframe');
        if (existingIframe) {
            // Set src to about:blank to stop the iframe's execution
            existingIframe.src = 'about:blank';
            // Remove the iframe from DOM
            existingIframe.remove();
        }
        streamContainer.innerHTML = '';
        streamContainer.style.display = 'none';
        noPreview.style.display = 'flex';
        
        console.log(`Stopped embedded stream for ${cameraId}`);
    }
    
    // Update master checkbox state
    setTimeout(updateMasterStreamCheckbox, 100);
}

function toggleStreamPreview(cameraId, cameraPath) {
    // Open stream in a modal or new window for better viewing
    openCameraStream(cameraId, cameraPath, false); // Preview doesn't require token check
}

async function toggleStream(cameraId, cameraPath) {
    // Note: This would require API endpoints to start/stop individual camera streams
    // For now, we'll just refresh the status to show current state
    showAlert('Stream control coming soon - use the streaming server controls for now', 'info');
    setTimeout(refreshStatus, 5000); // Increased delay to reduce CPU usage
}

function formatUptime(seconds) {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    
    const parts = [];
    if (days > 0) parts.push(`${days}d`);
    if (hours > 0) parts.push(`${hours}h`);
    if (minutes > 0) parts.push(`${minutes}m`);
    
    return parts.join(' ') || '< 1m';
}

function formatFileSize(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i];
}

function startAutoRefresh() {
    const autoRefresh = document.getElementById('autoRefresh').checked;
    if (autoRefresh) {
        const interval = parseInt(document.getElementById('refreshInterval').value);
        refreshTimer = setInterval(refreshStatus, interval);
    }
}

function stopAutoRefresh() {
    if (refreshTimer) {
        clearInterval(refreshTimer);
        refreshTimer = null;
    }
}

document.getElementById('autoRefresh').addEventListener('change', (e) => {
    if (e.target.checked) {
        startAutoRefresh();
    } else {
        stopAutoRefresh();
    }
});

document.getElementById('refreshInterval').addEventListener('change', () => {
    stopAutoRefresh();
    if (document.getElementById('autoRefresh').checked) {
        startAutoRefresh();
    }
});

// Set favicon
document.getElementById('favicon').href = `${basePath}/static/favicon.ico`;

// Recording functionality
async function toggleRecording(cameraId, cameraPath, requiresToken) {
    const headers = {
        'Content-Type': 'application/json'
    };
    
    if (requiresToken) {
        const tokenInput = document.getElementById(`token-${cameraId}`);
        if (tokenInput && tokenInput.value.trim()) {
            const token = tokenInput.value.trim();
            headers['Authorization'] = `Bearer ${token}`;
        } else {
            showAlert('Please enter a token for this camera', 'error');
            return;
        }
    }
    
    if (adminToken) {
        headers['Authorization'] = `Bearer ${adminToken}`;
    }
    
    try {
        // First check current recording status
        const statusResponse = await fetch(`${basePath}${cameraPath}/control/recording/active`, { headers });
        const statusData = await statusResponse.json();
        
        const isRecording = statusData.status === 'success' && statusData.data && statusData.data.active;
        const action = isRecording ? 'stop' : 'start';
        
        const requestBody = action === 'start' ? JSON.stringify({ reason: 'Manual recording started from dashboard' }) : undefined;
        
        const response = await fetch(`${basePath}${cameraPath}/control/recording/${action}`, {
            method: 'POST',
            headers,
            body: requestBody
        });
        
        if (response.ok) {
            showAlert(`Recording ${action}ed successfully`, 'success');
            // Update recording status for this camera
            updateRecordingStatus(cameraId, cameraPath, requiresToken);
        } else {
            const errorData = await response.text();
            showAlert(`Failed to ${action} recording: ${errorData}`, 'error');
        }
    } catch (error) {
        showAlert(`Error controlling recording: ${error.message}`, 'error');
    }
}

async function updateRecordingStatus(cameraId, cameraPath, requiresToken) {
    const headers = {};
    
    if (requiresToken) {
        const tokenInput = document.getElementById(`token-${cameraId}`);
        if (tokenInput && tokenInput.value.trim()) {
            const token = tokenInput.value.trim();
            headers['Authorization'] = `Bearer ${token}`;
        }
    }
    
    if (adminToken) {
        headers['Authorization'] = `Bearer ${adminToken}`;
    }
    
    try {
        // Get recording status
        const statusResponse = await fetch(`${basePath}${cameraPath}/control/recording/active`, { headers });
        
        if (statusResponse.ok) {
            const statusData = await statusResponse.json();
            
            const recordingStatusElement = document.getElementById(`recording-status-${cameraId}`);
            const recordingBtnElement = document.getElementById(`recording-btn-${cameraId}`);
            
            const isRecording = statusData.status === 'success' && statusData.data && statusData.data.active;
            
            if (recordingStatusElement) {
                recordingStatusElement.textContent = isRecording ? 'Active' : 'Stopped';
                recordingStatusElement.className = `recording-status-badge ${isRecording ? 'active' : 'stopped'}`;
            }
            
            if (recordingBtnElement) {
                recordingBtnElement.textContent = isRecording ? '⏹️ Stop Recording' : '🔴 Start Recording';
                recordingBtnElement.style.background = isRecording ? '#e74c3c' : '#27ae60';
                recordingBtnElement.style.display = 'inline-block'; // Show button since recording is available
                // Ensure compact styling is maintained
                recordingBtnElement.style.marginLeft = '10px';
                recordingBtnElement.style.padding = '2px 8px';
                recordingBtnElement.style.fontSize = '12px';
                recordingBtnElement.style.border = 'none';
                recordingBtnElement.style.borderRadius = '3px';
                recordingBtnElement.style.color = 'white';
                recordingBtnElement.style.cursor = 'pointer';
            }
        }
        
        // Get database size
        const dbSizeResponse = await fetch(`${basePath}${cameraPath}/control/recording/size`, { headers });
        const dbSizeData = await dbSizeResponse.json();
        
        const dbSizeElement = document.getElementById(`db-size-${cameraId}`);
        if (dbSizeElement && dbSizeData.status === 'success' && dbSizeData.data) {
            dbSizeElement.textContent = formatFileSize(dbSizeData.data.size_bytes);
        }
        
    } catch (error) {
        const recordingStatusElement = document.getElementById(`recording-status-${cameraId}`);
        const dbSizeElement = document.getElementById(`db-size-${cameraId}`);
        
        if (recordingStatusElement) {
            recordingStatusElement.textContent = 'Error';
            recordingStatusElement.className = 'recording-status-badge stopped';
        }
        
        if (dbSizeElement) {
            dbSizeElement.textContent = 'Error';
        }
    }
}


// Initial load
refreshStatus(true); // Force full rebuild on initial load
startAutoRefresh();