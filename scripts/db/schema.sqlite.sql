-- SQLite 建表（竞赛部署：零外部依赖、单文件持久化）。
-- 与 schema.sql(MySQL) 等价；差异：去 CHARSET、INDEX 拆为独立 CREATE INDEX、类型用 TEXT/INTEGER。

CREATE TABLE IF NOT EXISTS endpoints (
  id TEXT PRIMARY KEY, name TEXT, ip TEXT, mac TEXT,
  os_name TEXT, os_kind TEXT, cpu_model TEXT, cpu_arch TEXT,
  last_seen INTEGER
);

CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY, mode TEXT, from_id TEXT, to_id TEXT,
  start_at INTEGER, end_at INTEGER, status TEXT
);

CREATE TABLE IF NOT EXISTS audit_logs (
  id TEXT PRIMARY KEY, session_id TEXT, ts INTEGER,
  actor_id TEXT, event_type TEXT, text TEXT   -- event_type：避开保留字 type（裁决 B-DB1）
);
CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_logs (session_id);
CREATE INDEX IF NOT EXISTS idx_audit_ts ON audit_logs (ts);

-- 管理平台配置（key-value）：admin_user / admin_pass_hash 等，系统设置页可改
CREATE TABLE IF NOT EXISTS settings (
  k TEXT PRIMARY KEY, v TEXT
);
