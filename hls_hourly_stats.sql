-- HLS Hourly Storage Statistics
-- Shows HLS segment storage grouped by hour for the last 24 hours

SELECT 
    rs.camera_id,
    strftime('%Y-%m-%d %H:00:00', rh.start_time) as hour,
    COUNT(*) as segment_count,
    ROUND(SUM(rh.size_bytes) / 1024.0 / 1024.0, 2) as size_mb,
    ROUND(AVG(rh.size_bytes) / 1024.0, 2) as avg_segment_kb,
    ROUND(SUM(rh.duration_seconds) / 60.0, 1) as total_minutes,
    ROUND(AVG(rh.duration_seconds), 1) as avg_segment_seconds,
    ROUND(SUM(rh.size_bytes) / SUM(rh.duration_seconds), 0) as bytes_per_second,
    MIN(rh.start_time) as first_segment,
    MAX(rh.end_time) as last_segment
FROM recording_hls rh
JOIN recording_sessions rs ON rh.session_id = rs.id
WHERE rh.start_time >= datetime('now', '-24 hours')
GROUP BY rs.camera_id, strftime('%Y-%m-%d %H:00:00', rh.start_time)
ORDER BY rs.camera_id, hour DESC;