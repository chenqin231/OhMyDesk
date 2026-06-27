CREATE TABLE IF NOT EXISTS endpoints (
  id VARCHAR(64) PRIMARY KEY, name VARCHAR(128), ip VARCHAR(64), mac VARCHAR(32),
  os_name VARCHAR(128), os_kind VARCHAR(16), cpu_model VARCHAR(128), cpu_arch VARCHAR(16),
  last_seen BIGINT
) DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS sessions (
  id VARCHAR(64) PRIMARY KEY, mode CHAR(1), from_id VARCHAR(64), to_id VARCHAR(64),
  start_at BIGINT, end_at BIGINT, status VARCHAR(16)
) DEFAULT CHARSET=utf8mb4;

CREATE TABLE IF NOT EXISTS audit_logs (
  id VARCHAR(64) PRIMARY KEY, session_id VARCHAR(64), ts BIGINT,
  actor_id VARCHAR(64), event_type VARCHAR(16), text TEXT,   -- event_type：type 是 MySQL 保留字（裁决 B-DB1）
  INDEX idx_session (session_id), INDEX idx_ts (ts)
) DEFAULT CHARSET=utf8mb4;
