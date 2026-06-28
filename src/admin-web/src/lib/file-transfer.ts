// 文件传输 / 命令执行的浏览器侧辅助：分块 base64 编解码 + 触发下载 + id 生成。
// 与被控端 transfer.rs 对齐：CHUNK 64KB，data 为每块原始字节的 base64。

export const CHUNK_SIZE = 64 * 1024;
/** 命令默认超时（ms）；被控端封顶 120s。 */
export const EXEC_TIMEOUT_MS = 30_000;

export function genId(prefix: string): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** 原始字节 → base64（逐字符构造二进制串，避免 fromCharCode(...大数组) 触发参数上限）。 */
export function bytesToB64(bytes: Uint8Array): string {
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
  return btoa(bin);
}

/** base64 → 原始字节。 */
export function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** 把若干字节分片拼成 Blob 并触发浏览器下载。 */
export function downloadBytes(name: string, parts: Uint8Array[]): void {
  const blob = new Blob(parts as BlobPart[], { type: "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name || "download.bin";
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
