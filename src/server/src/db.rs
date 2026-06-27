//! MySQL 连接池。
//! M-SRV1：连接失败降级为 Option<Db>=None，实时链路不依赖 DB。

use sqlx::mysql::MySqlPoolOptions;

pub type Db = sqlx::MySqlPool;

/// 尝试连接 MySQL 并执行建表 DDL。
/// 返回 None 时调用方降级（审计 best-effort 跳过）。
pub async fn connect() -> Option<Db> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            tracing::warn!("DATABASE_URL 未设置，审计存储已降级（M-SRV1）");
            return None;
        }
    };

    let pool = match MySqlPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("MySQL 连接失败，审计存储已降级（M-SRV1）: {e}");
            return None;
        }
    };

    // 执行建表脚本（幂等 IF NOT EXISTS）
    let ddl = include_str!("../../../scripts/db/schema.sql");
    if let Err(e) = sqlx::raw_sql(ddl).execute(&pool).await {
        tracing::warn!("建表失败，审计存储已降级（M-SRV1）: {e}");
        return None;
    }

    tracing::info!("MySQL 连接成功，审计存储就绪");
    Some(pool)
}
