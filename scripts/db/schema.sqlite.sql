-- SQLite 建表（竞赛部署：零外部依赖、单文件持久化）。
-- 与 schema.sql(MySQL) 等价；差异：去 CHARSET、INDEX 拆为独立 CREATE INDEX、类型用 TEXT/INTEGER。

CREATE TABLE IF NOT EXISTS endpoints (
  id TEXT PRIMARY KEY, name TEXT, ip TEXT, mac TEXT,
  os_name TEXT, os_kind TEXT, cpu_model TEXT, cpu_arch TEXT,
  last_seen INTEGER
);

-- 终端注册表持久化（修复「服务器重启/升级后终端列表为空」）：
-- 内存 Registry 在 upsert/remove 时同步落库，启动时回灌（恢复为离线，agent 重连后转在线）。
-- info 存完整 EndpointInfo JSON（无损，避免结构化列丢字段）。
-- 刻意不落 password：模式 B 密码是每次客户端启动轮换的临时码，离线终端本就不可被控，
-- agent 重连会以新密码重注册覆盖内存值——故持久化密码既无用又徒增明文落盘泄露面。
CREATE TABLE IF NOT EXISTS endpoint_registry (
  id TEXT PRIMARY KEY,
  info TEXT NOT NULL,
  last_seen INTEGER NOT NULL,
  owner_id TEXT
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

-- 管理员登录日志（功能②）：记录每次登录尝试的 IP/UA/时间/成败
CREATE TABLE IF NOT EXISTS login_log (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  ts         INTEGER NOT NULL,          -- unix 秒
  username   TEXT    NOT NULL,          -- 尝试登录的用户名
  ip         TEXT,                       -- 客户端 IP（可空）
  user_agent TEXT,                       -- User-Agent（可空）
  success    INTEGER NOT NULL,          -- 1 成功 / 0 失败
  reason     TEXT                        -- 失败原因（成功为 NULL）
);
CREATE INDEX IF NOT EXISTS idx_login_log_ts ON login_log(ts);

-- 管理账户：tier 二值角色 + 按账户菜单权限集（替代旧 4 固定角色）。
-- 旧库由 db.rs::migrate_users_to_per_account_permissions 幂等迁移（role→tier + backfill permissions）。
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  -- tier：superadmin（隐式全权、独占账户管理）/ user（按 permissions 授菜单）
  role TEXT NOT NULL CHECK(role IN ('superadmin', 'user')),
  -- 按账户菜单权限键集（逗号分隔，如 'view_assets,use_remote'）；superadmin 留空表示隐式全权
  permissions TEXT NOT NULL DEFAULT '',
  enabled INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
