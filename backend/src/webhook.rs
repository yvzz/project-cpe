/*
 * @Author: 1orz cloudorzi@gmail.com
 * @Date: 2025-12-09 17:34:01
 * @LastEditors: 1orz cloudorzi@gmail.com
 * @LastEditTime: 2025-12-13 12:46:25
 * @FilePath: /udx710-backend/backend/src/webhook.rs
 * @Description: 
 * 
 * Copyright (c) 2025 by 1orz, All Rights Reserved. 
 */
//! Webhook 转发模块
//!
//! 支持五种互斥单选的通知渠道：钉钉、飞书、企业微信、邮件、Bark（iOS推送）
//! 各渠道使用各自的签名机制和 payload 格式

use crate::config::{ChannelType, DingtalkConfig, EmailConfig, FeishuConfig, NotificationChannel, WecomConfig, BarkConfig};
use crate::db::{CallRecord, SmsMessage};
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use std::fmt::Write as FmtWrite;
use std::sync::Arc;

type HmacSha256 = Hmac<sha2::Sha256>;

// ---------------------------------------------------------------------------
// 默认模板（各渠道内置 fallback）
// ---------------------------------------------------------------------------

const DEFAULT_DINGTALK_TEMPLATE: &str = r#"{"msgtype":"text","text":{"content":"📱 短信\n发送方: {{phone_number}}\n内容: {{content}}\n时间: {{timestamp}}"}}"#;

const DEFAULT_FEISHU_TEMPLATE: &str = r#"{"msg_type":"text","content":{"text":"📱 短信\n发送方: {{phone_number}}\n内容: {{content}}\n时间: {{timestamp}}"}}"#;

const DEFAULT_WECOM_TEMPLATE: &str = r#"{"msgtype":"text","content":{"content":"📱 短信\n发送方: {{phone_number}}\n内容: {{content}}\n时间: {{timestamp}}"}}"#;

const DEFAULT_DINGTALK_CALL_TEMPLATE: &str = r#"{"msgtype":"text","text":{"content":"📞 来电\n号码: {{phone_number}}\n类型: {{direction_cn}}\n时间: {{start_time}}\n时长: {{duration}}秒\n已接听: {{answered}}"}}"#;

const DEFAULT_FEISHU_CALL_TEMPLATE: &str = r#"{"msg_type":"text","content":{"text":"📞 来电\n号码: {{phone_number}}\n类型: {{direction_cn}}\n时间: {{start_time}}\n时长: {{duration}}秒\n已接听: {{answered}}"}}"#;

const DEFAULT_WECOM_CALL_TEMPLATE: &str = r#"{"msgtype":"text","content":{"content":"📞 来电\n号码: {{phone_number}}\n类型: {{direction_cn}}\n时间: {{start_time}}\n时长: {{duration}}秒\n已接听: {{answered}}"}}"#;

// ---------------------------------------------------------------------------
// WebhookSender
// ---------------------------------------------------------------------------

/// Webhook 发送器
pub struct WebhookSender {
    client: Client,
    config_manager: Arc<crate::config::ConfigManager>,
}

impl WebhookSender {
    /// 创建新的 Webhook 发送器
    pub fn new(config_manager: Arc<crate::config::ConfigManager>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("Failed to create HTTP client"),
            config_manager,
        }
    }
    
    /// 获取当前通知渠道配置
    fn get_config(&self) -> NotificationChannel {
        self.config_manager.get_webhook()
    }
    
    /// 转发短信
    pub async fn forward_sms(&self, message: &SmsMessage) -> Result<(), String> {
        let config = self.get_config();
        
        if !config.is_channel_enabled() || !config.forward_sms {
            return Ok(());
        }
        
        let payload = render_sms_for_channel(&config, message);
        self.send_by_channel(config.channel, &config, &payload).await
    }
    
    /// 转发通话记录
    pub async fn forward_call(&self, call: &CallRecord) -> Result<(), String> {
        let config = self.get_config();
        
        if !config.is_channel_enabled() || !config.forward_calls {
            return Ok(());
        }
        
        let payload = render_call_for_channel(&config, call);
        self.send_by_channel(config.channel, &config, &payload).await
    }
    
    /// 根据渠道类型分发发送
    async fn send_by_channel(
        &self,
        channel: ChannelType,
        config: &NotificationChannel,
        payload: &str,
    ) -> Result<(), String> {
        match channel {
            ChannelType::None => Ok(()),
            ChannelType::Dingtalk => self.send_dingtalk(&config.dingtalk, payload).await,
            ChannelType::Feishu => self.send_feishu(&config.feishu, payload).await,
            ChannelType::Wecom => self.send_wecom(&config.wecom, payload).await,
            ChannelType::Email => self.send_email(&config.email, payload).await,
            ChannelType::Bark => self.send_bark(&config.bark, payload).await,
        }
    }
    
    /// 测试通知渠道（发送测试消息）
    pub async fn test_webhook(&self) -> Result<String, String> {
        let config = self.get_config();
        
        if !config.is_channel_enabled() {
            return Err("No notification channel is enabled".to_string());
        }
        
        // 使用模拟短信数据测试
        let test_message = SmsMessage {
            id: 0,
            direction: "incoming".to_string(),
            phone_number: "+8613800138000".to_string(),
            content: "这是一条测试消息 (Webhook Test)".to_string(),
            timestamp: Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            status: "received".to_string(),
            pdu: None,
        };
        
        let payload = render_sms_for_channel(&config, &test_message);
        self.send_by_channel(config.channel, &config, &payload).await?;
        
        Ok(format!("Test message sent via {:?} successfully", config.channel))
    }
    
    // ---------------------------------------------------------------------------
    // 各渠道发送函数
    // ---------------------------------------------------------------------------
    
    /// 发送钉钉机器人消息
    /// 签名算法: timestamp\nsecret → HMAC-SHA256 → Base64 → URL参数 sign=
    async fn send_dingtalk(&self, cfg: &DingtalkConfig, payload: &str) -> Result<(), String> {
        if cfg.url.is_empty() {
            return Err("Dingtalk URL is not configured".to_string());
        }
        
        // 追加签名参数
        let final_url = if !cfg.secret.is_empty() {
            build_robot_signed_url(&cfg.url, &cfg.secret)
        } else {
            cfg.url.clone()
        };
        
        let response = self.client
            .post(&final_url)
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .map_err(|e| format!("Failed to send Dingtalk message: {}", e))?;
        
        check_response(response).await
    }
    
    /// 发送飞书机器人消息
    /// 签名算法: timestamp\nsecret → HMAC-SHA256 → Base64 → X-Feishu-Signature header
    async fn send_feishu(&self, cfg: &FeishuConfig, payload: &str) -> Result<(), String> {
        if cfg.url.is_empty() {
            return Err("Feishu URL is not configured".to_string());
        }
        
        let mut request = self.client
            .post(&cfg.url)
            .header("Content-Type", "application/json")
            .body(payload.to_string());
        
        // 如果配置了密钥，添加飞书签名头
        if !cfg.secret.is_empty() {
            let signature = compute_feishu_signature(&cfg.secret);
            request = request.header("X-Feishu-Signature", signature);
        }
        
        let response = request
            .send()
            .await
            .map_err(|e| format!("Failed to send Feishu message: {}", e))?;
        
        check_response(response).await
    }
    
    /// 发送企业微信机器人消息
    /// 签名算法同钉钉: timestamp\nsecret → HMAC-SHA256 → Base64 → URL参数 sign=
    async fn send_wecom(&self, cfg: &WecomConfig, payload: &str) -> Result<(), String> {
        if cfg.url.is_empty() {
            return Err("Wecom URL is not configured".to_string());
        }
        
        let final_url = if !cfg.secret.is_empty() {
            build_robot_signed_url(&cfg.url, &cfg.secret)
        } else {
            cfg.url.clone()
        };
        
        let response = self.client
            .post(&final_url)
            .header("Content-Type", "application/json")
            .body(payload.to_string())
            .send()
            .await
            .map_err(|e| format!("Failed to send Wecom message: {}", e))?;
        
        check_response(response).await
    }
    
    /// 发送邮件
    #[allow(clippy::unused_async)]
    async fn send_email(&self, cfg: &EmailConfig, payload: &str) -> Result<(), String> {
        use lettre::message::{Mailbox, MessageBuilder, MultiPart, SinglePart};
        use lettre::transport::smtp::client::TlsParameters;
        use lettre::transport::smtp::client::Tls as SmtpTls;
        use lettre::SmtpTransport;
        use lettre::Transport;
        use std::time::Duration;
        
        if cfg.smtp_host.is_empty() || cfg.username.is_empty() || cfg.to_addresses.is_empty() {
            return Err("Email config is incomplete".to_string());
        }
        
        // 解析收件人
        let to_addresses: Vec<&str> = cfg.to_addresses.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if to_addresses.is_empty() {
            return Err("No valid recipient email address".to_string());
        }
        
        // 从 payload 解析 subject 和 body（payload 格式: "SUBJECT\n\nBODY"）
        let (subject, body) = if let Some(pos) = payload.find("\n\n") {
            (payload[..pos].to_string(), payload[pos + 2..].to_string())
        } else {
            ("[CPE] 通知消息".to_string(), payload.to_string())
        };
        
        let subject = if cfg.subject_prefix.is_empty() {
            subject
        } else {
            format!("{} {}", cfg.subject_prefix.trim(), subject)
        };
        
        // 构建发件人 Mailbox
        let from_str = if cfg.from_name.is_empty() {
            cfg.username.clone()
        } else {
            format!("{} <{}>", cfg.from_name, cfg.username)
        };
        let from: Mailbox = from_str.parse()
            .map_err(|e| format!("Invalid from address: {}", e))?;
        
        // 解析第一个收件人
        let first_to: Mailbox = to_addresses[0].parse()
            .map_err(|e| format!("Invalid to address: {}", e))?;
        
        // 构建邮件
        let mut email_builder = MessageBuilder::new()
            .to(first_to)
            .subject(&subject)
            .from(from);
        
        // 添加抄送收件人
        for addr in to_addresses.iter().skip(1) {
            let cc: Mailbox = addr.parse().map_err(|e| format!("Invalid CC address: {}", e))?;
            email_builder = email_builder.cc(cc);
        }
        
        let email = email_builder
            .multipart(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(body.clone()))
                    .singlepart(SinglePart::html(body))
            )
            .map_err(|e| format!("Failed to build email: {}", e))?;
        
        // 提取域名用于 TLS 验证
        let tls_domain = cfg.smtp_host.split(':').next().unwrap_or(&cfg.smtp_host);
        let tls_params = TlsParameters::new(tls_domain.into())
            .map_err(|e| format!("Failed to create TLS parameters: {}", e))?;
        
        let transport = if cfg.use_tls {
            // SSL/TLS (port 465)
            SmtpTransport::relay(&cfg.smtp_host)
                .map_err(|e| format!("Failed to create SMTP relay: {}", e))?
                .port(cfg.smtp_port)
                .credentials((&cfg.username, &cfg.password).into())
                .tls(SmtpTls::Wrapper(tls_params))
                .timeout(Some(Duration::from_secs(10)))
                .build()
        } else {
            // STARTTLS (port 587)
            SmtpTransport::starttls_relay(&cfg.smtp_host)
                .map_err(|e| format!("Failed to create STARTTLS relay: {}", e))?
                .port(cfg.smtp_port)
                .credentials((&cfg.username, &cfg.password).into())
                .tls(SmtpTls::Required(tls_params))
                .timeout(Some(Duration::from_secs(10)))
                .build()
        };
        
        // 使用 tokio 阻塞 SMTP 发送
        let result = tokio::task::spawn_blocking(move || {
            transport.send(&email)
        })
        .await
        .map_err(|e| format!("Email task failed: {}", e))?
        .map_err(|e| format!("Failed to send email: {}", e))?;
        
        let _ = result;
        Ok(())
    }
    
    /// 发送 Bark iOS 推送
    async fn send_bark(&self, cfg: &BarkConfig, payload: &str) -> Result<(), String> {
        if cfg.device_key.is_empty() {
            return Err("Bark device key is not configured".to_string());
        }
        
        let server_url = if cfg.server_url.is_empty() {
            "https://api.day.app"
        } else {
            &cfg.server_url
        };
        
        // payload 格式: "TITLE\n\nBODY"
        let (title, body) = if let Some(pos) = payload.find("\n\n") {
            (payload[..pos].to_string(), payload[pos + 2..].to_string())
        } else {
            ("CPE 通知".to_string(), payload.to_string())
        };
        
        // 构建 Bark API URL
        let url = format!("{}/{}", server_url.trim_end_matches('/'), cfg.device_key);
        
        let mut query_params = vec![
            ("title", title.as_str()),
            ("body", body.as_str()),
        ];
        
        if !cfg.sound.is_empty() {
            query_params.push(("sound", &cfg.sound));
        }
        if !cfg.icon.is_empty() {
            query_params.push(("icon", &cfg.icon));
        }
        if !cfg.group.is_empty() {
            query_params.push(("group", &cfg.group));
        }
        
        let response = self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .query(&query_params)
            .send()
            .await
            .map_err(|e| format!("Failed to send Bark message: {}", e))?;
        
        check_response(response).await
    }
}

// ---------------------------------------------------------------------------
// 模板渲染
// ---------------------------------------------------------------------------

/// 根据渠道渲染短信内容
fn render_sms_for_channel(config: &NotificationChannel, sms: &SmsMessage) -> String {
    match config.channel {
        ChannelType::Dingtalk => {
            let tpl = if config.dingtalk.template.is_empty() {
                DEFAULT_DINGTALK_TEMPLATE
            } else {
                &config.dingtalk.template
            };
            render_template(tpl, sms, None)
        }
        ChannelType::Feishu => {
            let tpl = if config.feishu.template.is_empty() {
                DEFAULT_FEISHU_TEMPLATE
            } else {
                &config.feishu.template
            };
            render_template(tpl, sms, None)
        }
        ChannelType::Wecom => {
            let tpl = if config.wecom.template.is_empty() {
                DEFAULT_WECOM_TEMPLATE
            } else {
                &config.wecom.template
            };
            render_template(tpl, sms, None)
        }
        ChannelType::Email => {
            // 邮件格式: "SUBJECT\n\nBODY"
            let body = format!(
                "发送方: {}\n内容: {}\n时间: {}",
                sms.phone_number,
                sms.content,
                sms.timestamp
            );
            format!("[CPE短信] {}\n\n{}", sms.phone_number, body)
        }
        ChannelType::Bark => {
            let body = format!(
                "发送方: {}\n内容: {}\n时间: {}",
                sms.phone_number, sms.content, sms.timestamp
            );
            format!("📱 短信通知\n\n{}", body)
        }
        ChannelType::None => String::new(),
    }
}

/// 根据渠道渲染通话内容
fn render_call_for_channel(config: &NotificationChannel, call: &CallRecord) -> String {
    let direction_cn = if call.direction == "incoming" { "来电" } else { "去电" };
    let answered_str = if call.answered { "是" } else { "否" };
    
    match config.channel {
        ChannelType::Dingtalk => {
            let tpl = if config.dingtalk.template.is_empty() {
                DEFAULT_DINGTALK_CALL_TEMPLATE
            } else {
                &config.dingtalk.template
            };
            render_template(tpl, &crate::db::SmsMessage {
                id: 0, direction: String::new(), phone_number: String::new(),
                content: String::new(), timestamp: String::new(), status: String::new(), pdu: None,
            }, Some(call))
        }
        ChannelType::Feishu => {
            let tpl = if config.feishu.template.is_empty() {
                DEFAULT_FEISHU_CALL_TEMPLATE
            } else {
                &config.feishu.template
            };
            render_template(tpl, &crate::db::SmsMessage {
                id: 0, direction: String::new(), phone_number: String::new(),
                content: String::new(), timestamp: String::new(), status: String::new(), pdu: None,
            }, Some(call))
        }
        ChannelType::Wecom => {
            let tpl = if config.wecom.template.is_empty() {
                DEFAULT_WECOM_CALL_TEMPLATE
            } else {
                &config.wecom.template
            };
            render_template(tpl, &crate::db::SmsMessage {
                id: 0, direction: String::new(), phone_number: String::new(),
                content: String::new(), timestamp: String::new(), status: String::new(), pdu: None,
            }, Some(call))
        }
        ChannelType::Email => {
            let body = format!(
                "号码: {}\n类型: {}\n时间: {}\n时长: {}秒\n已接听: {}",
                call.phone_number,
                direction_cn,
                call.start_time,
                call.duration,
                answered_str
            );
            format!("[CPE来电] {}\n\n{}", call.phone_number, body)
        }
        ChannelType::Bark => {
            let body = format!(
                "号码: {}\n类型: {}\n时间: {}\n时长: {}秒\n已接听: {}",
                call.phone_number, direction_cn, call.start_time, call.duration, answered_str
            );
            format!("📞 来电通知\n\n{}", body)
        }
        ChannelType::None => String::new(),
    }
}

/// 通用模板替换，支持 {{变量名}} 格式
fn render_template(template: &str, sms: &SmsMessage, call: Option<&CallRecord>) -> String {
    let direction_cn = if sms.direction == "incoming" {
        "来电"
    } else if sms.direction == "outgoing" {
        "去电"
    } else {
        &sms.direction
    };
    
    let mut result = template.to_string();
    
    // 短信变量
    result = result.replace("{{phone_number}}", &sms.phone_number);
    result = result.replace("{{content}}", &escape_json_string(&sms.content));
    result = result.replace("{{timestamp}}", &sms.timestamp);
    result = result.replace("{{direction}}", &sms.direction);
    result = result.replace("{{direction_cn}}", direction_cn);
    result = result.replace("{{status}}", &sms.status);
    result = result.replace("{{id}}", &sms.id.to_string());
    // 别名
    result = result.replace("{{sender}}", &sms.phone_number);
    result = result.replace("{{message}}", &escape_json_string(&sms.content));
    result = result.replace("{{time}}", &sms.timestamp);
    
    // 通话变量
    if let Some(c) = call {
        result = result.replace("{{phone_number}}", &c.phone_number);
        result = result.replace("{{direction}}", &c.direction);
        let dc = if c.direction == "incoming" { "来电" } else { "去电" };
        result = result.replace("{{direction_cn}}", dc);
        result = result.replace("{{duration}}", &c.duration.to_string());
        result = result.replace("{{start_time}}", &c.start_time);
        result = result.replace("{{end_time}}", c.end_time.as_deref().unwrap_or(""));
        let ans = if c.answered { "是" } else { "否" };
        result = result.replace("{{answered}}", ans);
        result = result.replace("{{answered_bool}}", &c.answered.to_string());
        result = result.replace("{{id}}", &c.id.to_string());
        // 别名
        result = result.replace("{{caller}}", &c.phone_number);
        result = result.replace("{{time}}", &c.start_time);
    }
    
    result
}

/// 转义 JSON 字符串中的特殊字符
fn escape_json_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

// ---------------------------------------------------------------------------
// 签名计算
// ---------------------------------------------------------------------------

/// 计算钉钉/企微机器人签名
/// 算法: timestamp\nsecret → HMAC-SHA256 → Base64
fn compute_robot_sign(secret: &str) -> (i64, String) {
    use base64::engine::general_purpose::STANDARD;
    
    let timestamp = chrono::Utc::now().timestamp_millis();
    let string_to_sign = format!("{}\n{}", timestamp, secret);
    
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(string_to_sign.as_bytes());
    let result = mac.finalize().into_bytes();
    
    let sign = STANDARD.encode(&result);
    (timestamp, sign)
}

/// 为钉钉/企微机器人 URL 追加签名参数
fn build_robot_signed_url(url: &str, secret: &str) -> String {
    let (timestamp, sign) = compute_robot_sign(secret);
    let separator = if url.contains('?') { "&" } else { "?" };
    let encoded_sign = url_encode(&sign);
    format!("{}{}timestamp={}&sign={}", url, separator, timestamp, encoded_sign)
}

/// 计算飞书签名
/// 算法: timestamp\nsecret → HMAC-SHA256 → Base64 → X-Feishu-Signature header
fn compute_feishu_signature(secret: &str) -> String {
    use base64::engine::general_purpose::STANDARD;
    
    let timestamp = chrono::Utc::now().timestamp_millis();
    let string_to_sign = format!("{}\n{}", timestamp, secret);
    
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(string_to_sign.as_bytes());
    let result = mac.finalize().into_bytes();
    
    STANDARD.encode(&result)
}

/// URL 编码
fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 3);
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                encoded.push(ch);
            }
            '+' => encoded.push_str("%2B"),
            '/' => encoded.push_str("%2F"),
            '=' => encoded.push_str("%3D"),
            _ => {
                for byte in ch.to_string().as_bytes() {
                    write!(&mut encoded, "%{:02X}", byte).unwrap();
                }
            }
        }
    }
    encoded
}

// ---------------------------------------------------------------------------
// 响应检查
// ---------------------------------------------------------------------------

/// 检查 HTTP 响应
async fn check_response(response: reqwest::Response) -> Result<(), String> {
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(format!("Request failed with status {}: {}", status, body))
    }
}
