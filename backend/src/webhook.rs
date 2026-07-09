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
use sha2::Sha256;
use std::fmt::Write as FmtWrite;
use std::sync::{Arc, RwLock};

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// 默认模板（各渠道内置 fallback）
// ---------------------------------------------------------------------------

const DEFAULT_DINGTALK_TEMPLATE: &str = r#"{"msgtype":"text","text":{"content":"📱 短信\n发送方: {{phone_number}}\n接收方: {{self_number}}\n内容: {{content}}\n时间: {{local_time}}"}}"#;

const DEFAULT_FEISHU_TEMPLATE: &str = r#"{"msg_type":"text","content":{"text":"📱 短信\n发送方: {{phone_number}}\n接收方: {{self_number}}\n内容: {{content}}\n时间: {{local_time}}"}}"#;

const DEFAULT_WECOM_TEMPLATE: &str = r#"{"msgtype":"text","content":{"content":"📱 短信\n发送方: {{phone_number}}\n接收方: {{self_number}}\n内容: {{content}}\n时间: {{local_time}}"}}"#;

const DEFAULT_DINGTALK_CALL_TEMPLATE: &str = r#"{"msgtype":"text","text":{"content":"📞 来电\n号码: {{phone_number}}\n时间: {{local_time}}\n时长: {{duration}}秒"}}"#;

const DEFAULT_FEISHU_CALL_TEMPLATE: &str = r#"{"msg_type":"text","content":{"text":"📞 来电\n号码: {{phone_number}}\n时间: {{local_time}}\n时长: {{duration}}秒"}}"#;

const DEFAULT_WECOM_CALL_TEMPLATE: &str = r#"{"msgtype":"text","content":{"content":"📞 来电\n号码: {{phone_number}}\n时间: {{local_time}}\n时长: {{duration}}秒"}}"#;

// ---------------------------------------------------------------------------
// WebhookSender
// ---------------------------------------------------------------------------

/// Webhook 发送器
pub struct WebhookSender {
    client: Client,
    config_manager: Arc<crate::config::ConfigManager>,
    /// 缓存的本机号码（从 ofono 获取）
    self_number: RwLock<String>,
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
            self_number: RwLock::new(String::new()),
        }
    }

    /// 设置本机号码（启动时从 ofono 获取后调用）
    pub fn set_self_number(&self, number: &str) {
        let mut s = self.self_number.write().unwrap();
        *s = number.to_string();
    }

    /// 获取本机号码
    fn get_self_number(&self) -> String {
        self.self_number.read().unwrap().clone()
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
        
        let self_number = self.get_self_number();
        let payload = render_sms_for_channel(&config, message, &self_number);
        self.send_by_channel(config.channel, &config, &payload).await
    }
    
    /// 转发通话记录
    pub async fn forward_call(&self, call: &CallRecord) -> Result<(), String> {
        let config = self.get_config();
        
        if !config.is_channel_enabled() || !config.forward_calls {
            return Ok(());
        }
        
        let self_number = self.get_self_number();
        let payload = render_call_for_channel(&config, call, &self_number);
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
        
        let self_number = self.get_self_number();
        let payload = render_sms_for_channel(&config, &test_message, &self_number);
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
    async fn send_feishu(&self, cfg: &FeishuConfig, payload: &str) -> Result<(), String> {
        if cfg.url.is_empty() {
            return Err("Feishu URL is not configured".to_string());
        }
        
        let mut request = self.client
            .post(&cfg.url)
            .header("Content-Type", "application/json")
            .body(payload.to_string());
        
        if !cfg.secret.is_empty() {
            let signature = compute_feishu_signature(&cfg.secret);
            request = request.header("X-Feishu-Signature", signature);
        }
        
        let response = request.send().await
            .map_err(|e| format!("Failed to send Feishu message: {}", e))?;
        
        check_response(response).await
    }
    
    /// 发送企业微信机器人消息
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
    async fn send_email(&self, cfg: &EmailConfig, payload: &str) -> Result<(), String> {
        use lettre::message::{Mailbox, MessageBuilder, MultiPart, SinglePart};
        use lettre::transport::smtp::client::{Tls, TlsParameters};
        use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
        use std::time::Duration;

        if cfg.smtp_host.is_empty() || cfg.username.is_empty() || cfg.to_addresses.is_empty() {
            return Err("Email config is incomplete".to_string());
        }

        let to_addresses: Vec<&str> = cfg.to_addresses.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        if to_addresses.is_empty() {
            return Err("No valid recipient email address".to_string());
        }

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

        let from_str = if cfg.from_name.is_empty() {
            cfg.username.clone()
        } else {
            format!("{} <{}>", cfg.from_name, cfg.username)
        };
        let from: Mailbox = from_str.parse()
            .map_err(|e| format!("Invalid from address: {}", e))?;

        let first_to: Mailbox = to_addresses[0].parse()
            .map_err(|e| format!("Invalid to address: {}", e))?;

        let mut email_builder = MessageBuilder::new()
            .to(first_to)
            .subject(&subject)
            .from(from);

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

        let tls = if cfg.use_tls {
            let tls_params = TlsParameters::builder(cfg.smtp_host.clone())
                .build_rustls()
                .map_err(|e| format!("Failed to create TLS params: {}", e))?;
            Tls::Wrapper(tls_params)
        } else {
            let tls_params = TlsParameters::builder(cfg.smtp_host.clone())
                .build_rustls()
                .map_err(|e| format!("Failed to create TLS params: {}", e))?;
            Tls::Required(tls_params)
        };

        let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host)
            .port(cfg.smtp_port)
            .credentials((&cfg.username, &cfg.password).into())
            .tls(tls)
            .timeout(Some(Duration::from_secs(10)))
            .build();
        
        transport.send(email).await
            .map_err(|e| format!("Failed to send email: {}", e))?;

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
        
        let (title, body) = if let Some(pos) = payload.find("\n\n") {
            (payload[..pos].to_string(), payload[pos + 2..].to_string())
        } else {
            ("CPE 通知".to_string(), payload.to_string())
        };
        
        let url = format!("{}/{}", server_url.trim_end_matches('/'), cfg.device_key);
        
        let mut query_params = vec![
            ("title", title.as_str()),
            ("body", body.as_str()),
        ];
        if !cfg.sound.is_empty() { query_params.push(("sound", &cfg.sound)); }
        if !cfg.icon.is_empty() { query_params.push(("icon", &cfg.icon)); }
        if !cfg.group.is_empty() { query_params.push(("group", &cfg.group)); }
        
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

/// 将 UTC 时间字符串转换为北京时间 `yyyy-MM-dd HH:mm:ss`
fn utc_to_local(utc_str: &str) -> String {
    // 尝试解析 RFC3339 格式
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(utc_str) {
        let beijing = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
        return dt.with_timezone(&beijing).format("%Y-%m-%d %H:%M:%S").to_string();
    }
    // 回退：直接返回原值
    utc_str.to_string()
}

/// 根据渠道渲染短信内容
fn render_sms_for_channel(config: &NotificationChannel, sms: &SmsMessage, self_number: &str) -> String {
    let local_time = utc_to_local(&sms.timestamp);
    match config.channel {
        ChannelType::Dingtalk => {
            let tpl = if config.dingtalk.template.is_empty() { DEFAULT_DINGTALK_TEMPLATE } else { &config.dingtalk.template };
            render_template(tpl, sms, None, self_number, &local_time)
        }
        ChannelType::Feishu => {
            let tpl = if config.feishu.template.is_empty() { DEFAULT_FEISHU_TEMPLATE } else { &config.feishu.template };
            render_template(tpl, sms, None, self_number, &local_time)
        }
        ChannelType::Wecom => {
            let tpl = if config.wecom.template.is_empty() { DEFAULT_WECOM_TEMPLATE } else { &config.wecom.template };
            render_template(tpl, sms, None, self_number, &local_time)
        }
        ChannelType::Email => {
            let body = format!(
                "发送方: {}\n接收方: {}\n内容: {}\n时间: {}",
                sms.phone_number, self_number, sms.content, local_time
            );
            format!("[CPE短信] {}\n\n{}", sms.phone_number, body)
        }
        ChannelType::Bark => {
            let body = format!(
                "发送方: {}\n接收方: {}\n内容: {}\n时间: {}",
                sms.phone_number, self_number, sms.content, local_time
            );
            format!("📱 短信通知\n\n{}", body)
        }
        ChannelType::None => String::new(),
    }
}

/// 根据渠道渲染通话内容
fn render_call_for_channel(config: &NotificationChannel, call: &CallRecord, self_number: &str) -> String {
    let local_time = utc_to_local(&call.start_time);
    
    // 构建一个空 SMS 用于复用 render_template 的通话变量替换
    let dummy_sms = SmsMessage {
        id: 0, direction: String::new(), phone_number: String::new(),
        content: String::new(), timestamp: String::new(), status: String::new(), pdu: None,
    };
    
    match config.channel {
        ChannelType::Dingtalk => {
            let tpl = if config.dingtalk.template.is_empty() { DEFAULT_DINGTALK_CALL_TEMPLATE } else { &config.dingtalk.template };
            render_template(tpl, &dummy_sms, Some(call), self_number, &local_time)
        }
        ChannelType::Feishu => {
            let tpl = if config.feishu.template.is_empty() { DEFAULT_FEISHU_CALL_TEMPLATE } else { &config.feishu.template };
            render_template(tpl, &dummy_sms, Some(call), self_number, &local_time)
        }
        ChannelType::Wecom => {
            let tpl = if config.wecom.template.is_empty() { DEFAULT_WECOM_CALL_TEMPLATE } else { &config.wecom.template };
            render_template(tpl, &dummy_sms, Some(call), self_number, &local_time)
        }
        ChannelType::Email => {
            let body = format!(
                "号码: {}\n时间: {}\n时长: {}秒",
                call.phone_number, local_time, call.duration
            );
            format!("[CPE来电] {}\n\n{}", call.phone_number, body)
        }
        ChannelType::Bark => {
            let body = format!(
                "号码: {}\n时间: {}\n时长: {}秒",
                call.phone_number, local_time, call.duration
            );
            format!("📞 来电通知\n\n{}", body)
        }
        ChannelType::None => String::new(),
    }
}

/// 通用模板替换，支持 {{变量名}} 格式
fn render_template(template: &str, sms: &SmsMessage, call: Option<&CallRecord>, self_number: &str, local_time: &str) -> String {
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
    // 本机号码 + 本地时间
    result = result.replace("{{self_number}}", self_number);
    result = result.replace("{{local_time}}", local_time);
    
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
        result = result.replace("{{caller}}", &c.phone_number);
        result = result.replace("{{time}}", &c.start_time);
        result = result.replace("{{self_number}}", self_number);
        result = result.replace("{{local_time}}", local_time);
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

fn compute_robot_sign(secret: &str) -> (i64, String) {
    use base64::engine::general_purpose::STANDARD;
    let timestamp = chrono::Utc::now().timestamp_millis();
    let string_to_sign = format!("{}\n{}", timestamp, secret);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(string_to_sign.as_bytes());
    let sign = STANDARD.encode(&mac.finalize().into_bytes());
    (timestamp, sign)
}

fn build_robot_signed_url(url: &str, secret: &str) -> String {
    let (timestamp, sign) = compute_robot_sign(secret);
    let separator = if url.contains('?') { "&" } else { "?" };
    format!("{}{}timestamp={}&sign={}", url, separator, timestamp, url_encode(&sign))
}

fn compute_feishu_signature(secret: &str) -> String {
    use base64::engine::general_purpose::STANDARD;
    let timestamp = chrono::Utc::now().timestamp_millis();
    let string_to_sign = format!("{}\n{}", timestamp, secret);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(string_to_sign.as_bytes());
    STANDARD.encode(&mac.finalize().into_bytes())
}

fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 3);
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => encoded.push(ch),
            '+' => encoded.push_str("%2B"),
            '/' => encoded.push_str("%2F"),
            '=' => encoded.push_str("%3D"),
            _ => { write!(&mut encoded, "%{:02X}", ch as u8).unwrap(); }
        }
    }
    encoded
}

// ---------------------------------------------------------------------------
// 响应检查
// ---------------------------------------------------------------------------

async fn check_response(response: reqwest::Response) -> Result<(), String> {
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(format!("Request failed with status {}: {}", status, body))
    }
}
