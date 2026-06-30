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

-- 管理平台配置（key-value）：admin_user / admin_pass_hash 等可在系统设置页修改
CREATE TABLE IF NOT EXISTS settings (
  k VARCHAR(64) PRIMARY KEY, v TEXT
) DEFAULT CHARSET=utf8mb4;

-- 管理员登录日志（功能②）
CREATE TABLE IF NOT EXISTS login_log (
  id         BIGINT AUTO_INCREMENT PRIMARY KEY,
  ts         BIGINT NOT NULL,
  username   VARCHAR(128) NOT NULL,
  ip         VARCHAR(64),
  user_agent VARCHAR(512),
  success    TINYINT NOT NULL,
  reason     VARCHAR(255),
  INDEX idx_login_log_ts (ts)
);
