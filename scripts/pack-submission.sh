#!/usr/bin/env bash
# 打包参赛提交 ZIP：源码 + 文档 + 安装包，排除所有编译中间产物。
# 用法：bash scripts/pack-submission.sh [输出目录]
# 产出：<输出目录>/ohmydesk-submission-<日期>.zip（默认放仓库根）
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DATE="$(date +%Y%m%d)"
OUT_DIR="${1:-$REPO}"
ZIP_NAME="ohmydesk-submission-${DATE}.zip"
ZIP_PATH="$OUT_DIR/$ZIP_NAME"

cd "$REPO"

# 如果已有同名文件先删掉
rm -f "$ZIP_PATH"

echo "╔══════════════════════════════════════════╗"
echo "  OhMyDesk 提交包打包"
echo "  仓库：$REPO"
echo "  产出：$ZIP_PATH"
echo "╚══════════════════════════════════════════╝"
echo ""

# ── 包含哪些内容 ────────────────────────────────────────────────────────────
# 显式列出顶级目录/文件，而非用 * 再排除，避免误带 proto/ chat/ 等不开源内容。

INCLUDE=(
  # 源码
  src/
  # 根级配置（workspace 定义 + 依赖锁 + 容器）
  Cargo.toml
  Cargo.lock
  Dockerfile
  .dockerignore
  # CI/CD
  .github/
  # 文档
  docs/
  README.md
  # 资产
  assets/
  # 构建脚本
  scripts/
  # 发布产物（若已编译）
  dist/
  # AI 工具指令（评委可了解开发方式）
  .agent/
  AGENTS.md
  GEMINI.md
)

# ── 排除模式（编译中间产物 + 运行时文件 + 敏感信息）──────────────────────
EXCLUDES=(
  # Rust 编译缓存
  "target/*"
  # 前端/Node 中间产物
  "*/node_modules/*"
  # 前端构建输出（admin-web 和 mcp 各自的 dist，非根级 dist）
  "src/admin-web/dist/*"
  "src/mcp/dist/*"
  "*.tsbuildinfo"
  "*/.vite/*"
  # ts-rs 默认导出（冗余，真实类型在 src/admin-web/src/lib/types/）
  "src/protocol/bindings/*"
  # 运行时数据库
  "*.db"
  "*.db-wal"
  "*.db-shm"
  # 环境变量与密钥
  ".env"
  ".env.*"
  "*.key"
  "*.pem"
  # 杂项
  "*.log"
  ".DS_Store"
  "__pycache__/*"
)

# 构造 zip 排除参数
EXCLUDE_ARGS=()
for pat in "${EXCLUDES[@]}"; do
  EXCLUDE_ARGS+=("--exclude=$pat")
done

# 过滤出实际存在的包含项
REAL_INCLUDE=()
for item in "${INCLUDE[@]}"; do
  if [ -e "$item" ]; then
    REAL_INCLUDE+=("$item")
  else
    echo "  ⚠  跳过（不存在）: $item"
  fi
done

echo "打包中..."
zip -r "$ZIP_PATH" "${REAL_INCLUDE[@]}" "${EXCLUDE_ARGS[@]}" -q

# ── 统计 ────────────────────────────────────────────────────────────────────
TOTAL=$(zip -sf "$ZIP_PATH" | tail -1 | grep -oE '[0-9]+' | head -1 || true)
SIZE=$(du -sh "$ZIP_PATH" | cut -f1)

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  ✓ 打包完成"
echo "  文件：$ZIP_PATH"
echo "  大小：$SIZE"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "包含的顶级目录/文件："
zip -sf "$ZIP_PATH" | grep -E "^  [^/]+/?$" | sed 's/^/  /' || true
echo ""
echo "文件数统计（按类型）："
echo "  .rs    $(zip -sf "$ZIP_PATH" | grep -c '\.rs$' || echo 0) 个"
echo "  .tsx   $(zip -sf "$ZIP_PATH" | grep -c '\.tsx$' || echo 0) 个"
echo "  .ts    $(zip -sf "$ZIP_PATH" | grep -c '\.ts$' || echo 0) 个"
echo "  .md    $(zip -sf "$ZIP_PATH" | grep -c '\.md$' || echo 0) 个"
echo "  .sh    $(zip -sf "$ZIP_PATH" | grep -c '\.sh$' || echo 0) 个"
echo "  .toml  $(zip -sf "$ZIP_PATH" | grep -c '\.toml$' || echo 0) 个"
