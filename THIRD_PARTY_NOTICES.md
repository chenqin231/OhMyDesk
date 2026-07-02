# 第三方开源软件声明 (Third-Party Notices)

OhMyDesk 客户端在分发的二进制中包含以下开源软件。特此致谢并声明相应许可。

## Independent JPEG Group (IJG) —— 强制声明

本软件的 JPEG 编码经由 `jpeg-encoder` crate 实现，其中的 DCT 代码派生自
Independent JPEG Group 的工作。按 IJG 许可要求，随二进制分发的文档必须包含以下声明：

> **This software is based in part on the work of the Independent JPEG Group.**
> （本软件部分基于 Independent JPEG Group 的工作。）

- `jpeg-encoder` 0.7 — 许可：`(MIT OR Apache-2.0) AND IJG`

## 其他主要开源组件

下列 crate 以 MIT、Apache-2.0 或 BSD 类许可分发，版权归各自作者所有：

- `slint` 1.17 — GUI 框架（软件渲染）
- `fast_image_resize` 6 — SIMD 图像缩放
- `image` 0.25 — 图像解码
- `enigo` 0.6 — 键鼠注入
- `xcap` — 屏幕采集
- `tokio` / `tokio-tungstenite` — 异步运行时与 WebSocket
- `serde` / `serde_json` — 序列化
- `arboard` — 剪贴板
- `minisign-verify` — 更新包验签
- `native-tls` / `rustls` / `ureq` — TLS 与 HTTP
- `anyhow`、`base64`、`sha2`、`semver`、`url`、`uuid`、`rand`、`directories`、
  `sysinfo`、`tracing` 系列、`self-replace`、`tempfile` 等基础库

各组件完整许可文本见其上游仓库。如需逐项 SPDX 清单，可用 `cargo license` 生成。
