-- =============================================================================
-- HLS HOURLY STORAGE STATISTICS
-- Shows HLS segment storage grouped by hour for the last 24 hours
-- =============================================================================

SELECT 
    '=== HLS HOURLY STATS (Last 24 Hours) ===' as query_type,
    '' as camera_id,
    '' as hour,
    '' as segment_count,
    '' as size_mb,
    '' as avg_segment_size,
    '' as total_minutes,
    '' as avg_duration,
    '' as bytes_per_second,
    '' as first_segment,
    '' as last_segment

UNION ALL

SELECT 
    'HLS' as query_type,
    rs.camera_id,
    strftime('%Y-%m-%d %H:00:00', rh.start_time) as hour,
    COUNT(*) as segment_count,
    ROUND(SUM(rh.size_bytes) / 1024.0 / 1024.0, 2) as size_mb,
    ROUND(AVG(rh.size_bytes) / 1024.0, 2) || ' KB' as avg_segment_size,
    ROUND(SUM(rh.duration_seconds) / 60.0, 1) as total_minutes,
    ROUND(AVG(rh.duration_seconds), 1) || 's' as avg_duration,
    ROUND(SUM(rh.size_bytes) / SUM(rh.duration_seconds), 0) as bytes_per_second,
    MIN(rh.start_time) as first_segment,
    MAX(rh.end_time) as last_segment
FROM recording_hls rh
JOIN recording_sessions rs ON rh.session_id = rs.id
WHERE rh.start_time >= datetime('now', '-24 hours')
GROUP BY rs.camera_id, strftime('%Y-%m-%d %H:00:00', rh.start_time)

UNION ALL

-- Spacer
SELECT 
    '', '', '', '', '', '', '', '', '', '', ''

UNION ALL

-- =============================================================================
-- MP4 HOURLY STORAGE STATISTICS  
-- Shows MP4 segment storage grouped by hour for the last 24 hours
-- =============================================================================

SELECT 
    '=== MP4 HOURLY STATS (Last 24 Hours) ===' as query_type,
    '' as camera_id,
    '' as hour,
    '' as segment_count,
    '' as size_mb,
    '' as avg_segment_size,
    '' as total_minutes,
    '' as avg_duration,
    '' as bytes_per_second,
    '' as first_segment,
    '' as last_segment

UNION ALL

SELECT 
    'MP4' as query_type,
    rs.camera_id,
    strftime('%Y-%m-%d %H:00:00', rm.start_time) as hour,
    COUNT(*) as segment_count,
    ROUND(SUM(rm.size_bytes) / 1024.0 / 1024.0, 2) as size_mb,
    ROUND(AVG(rm.size_bytes) / 1024.0 / 1024.0, 2) || ' MB' as avg_segment_size,
    ROUND(SUM((julianday(rm.end_time) - julianday(rm.start_time)) * 1440), 1) as total_minutes,
    ROUND(AVG((julianday(rm.end_time) - julianday(rm.start_time)) * 1440), 1) || 'm' as avg_duration,
    ROUND(SUM(rm.size_bytes) / SUM((julianday(rm.end_time) - julianday(rm.start_time)) * 86400), 0) as bytes_per_second,
    MIN(rm.start_time) as first_segment,
    MAX(rm.end_time) as last_segment
FROM recording_mp4 rm
JOIN recording_sessions rs ON rm.session_id = rs.id
WHERE rm.start_time >= datetime('now', '-24 hours')
GROUP BY rs.camera_id, strftime('%Y-%m-%d %H:00:00', rm.start_time)

ORDER BY query_type DESC, camera_id, hour DESC;