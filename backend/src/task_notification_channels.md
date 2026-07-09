# 通知渠道重构完成报告

## 任务
将 project-cpe 的通知推送系统从单一 Webhook 改造为支持五种互斥单选渠道：钉钉、飞书、企业微信、邮件、Bark（iOS推送）。

## 修改的文件

### 1. `Cargo.toml`
新增依赖：
```toml
lettre = "0.11"
native-tls = "0.2"
```

### 2. `models.rs`
在文件末尾追加通知渠道相关类型，并 re-export 自 `config`：
```rust
// re-export 使其同时可通过 crate::models::ChannelType 访问
#[allow(unused_imports)]
pub use crate::config::{
    BarkConfig, ChannelType, DingtalkConfig, EmailConfig, FeishuConfig,
    NotificationChannel, WecomConfig,
};
```

### 3. `config.rs`（核心改动）
- **完全重写 `WebhookConfig` → `NotificationChannel`**
- 新增 `ChannelType` 枚举：`None | Dingtalk | Feishu | Wecom | Email | Bark`
- 新增各渠道配置结构：
  - `DingtalkConfig`：`url`, `secret`, `template`
  - `FeishuConfig`：`url`, `secret`, `template`
  - `WecomConfig`：`url`, `secret`, `template`
  - `EmailConfig`：`smtp_host`, `smtp_port`, `use_tls`, `username`, `password`, `from_name`, `to_addresses`, `subject_prefix`
  - `BarkConfig`：`server_url`, `device_key`, `sound`, `icon`, `group`
- `NotificationChannel`：互斥单选结构，包含各渠道配置、全局开关 `forward_sms/forward_calls`
- 新增辅助方法：
  - `is_channel_enabled(&self) -> bool`
  - `get_active_url(&self) -> Option<(ChannelType, &str)>`

### 4. `webhook.rs`（核心改动）
- 完全重写，移除旧的 `WebhookSender`
- `WebhookSender` 新增 `send_by_channel()` 分发方法
- 新增各渠道发送函数：
  - `send_dingtalk(cfg, payload)`：钉钉签名（HMAC-SHA256 + Base64，sign 放 URL 参数）
  - `send_feishu(cfg, payload)`：飞书签名（X-Feishu-Signature header）
  - `send_wecom(cfg, payload)`：企微签名（同钉钉，sign 放 URL 参数）
  - `send_email(cfg, payload)`：使用 lettre 0.11 发送邮件（SSL/TLS 或 STARTTLS）
  - `send_bark(cfg, payload)`：Bark iOS 推送（GET 请求带 query 参数）
- 新增通用模板渲染函数 `render_template()`，支持 `{{变量名}}` 替换
- 各渠道独立渲染函数：`render_sms_for_channel()`, `render_call_for_channel()`
- 内置默认模板（各渠道 fallback）
- 删除：`escape_json_string()` 独立函数、`compute_hmac_hex()`、`is_dingtalk_url()` 等旧逻辑

### 5. `handlers.rs`
- `get_webhook_config_handler` 返回类型改为 `crate::config::NotificationChannel`
- `set_webhook_config_handler` 接收类型改为 `crate::config::NotificationChannel`
- API 路径保持不变：`/api/webhook/config`（GET/POST）、`/api/webhook/test`（POST）

## 关键实现细节

### 钉钉/企微签名算法
```
timestamp = 当前毫秒时间戳
string_to_sign = "{timestamp}\n{secret}"
sign = Base64(HMAC-SHA256(secret, string_to_sign))
最终 URL: {url}?timestamp={timestamp}&sign={url_encoded(sign)}
```

### 飞书签名算法
```
timestamp = 当前毫秒时间戳
string_to_sign = "{timestamp}\n{secret}"
signature = Base64(HMAC-SHA256(secret, string_to_sign))
Header: X-Feishu-Signature: {signature}
```

### 邮件发送
- 使用 `lettre 0.11` + `native-tls`
- SSL/TLS（port 465）：`SmtpTransport::relay()` + `Tls::Wrapper`
- STARTTLS（port 587）：`SmtpTransport::starttls_relay()` + `Tls::Required`
- 所有 SMTP 调用通过 `spawn_blocking` 封装为异步

### Bark 推送
- POST `https://api.day.app/{device_key}/`
- Query 参数：`title`, `body`, `sound`, `icon`, `group`

## 编译验证
```
cargo check --all-targets  # ✅ 零警告通过
```
