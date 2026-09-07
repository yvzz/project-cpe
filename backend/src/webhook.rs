/*
 * @Author: 1orz cloudorzi@gmail.com
 * @Date: 2025-12-09 17:34:01
 * @LastEditors: 1orz cloudorzi@gmail.com
 * @LastEditTime: 2026-07-26 00:49:00
 * @FilePath: /udx710-backend/backend/src/webhook.rs
 * @Description: Webhook 转发模块
 * 
 * Copyright (c) 2025 by 1orz, All Rights Reserved. 
 */
//! Webhook 转发模块
//!
//! 支持九种互斥单选的通知渠道：钉钉、飞书、企业微信、邮件、Bark（iOS推送）、
//! PushPlus、Server酱、PushDeer、ntfy（后四种吸收自上游短信推送体系）
//! 各渠道使用各自的签名机制和 payload 格式

use crate::config::{BarkConfig, ChannelType, DingtalkConfig, EmailConfig, FeishuConfig, NotificationChannel, PushProviderConfig, WecomConfig};
use crate::db::{CallRecord, SmsMessage};
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::{json, Value};
use sha2::Sha256;
use std::fmt::Write as FmtWrite;
use std::sync::{Arc, RwLock};

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// 默认模板（各渠道内置 fallback）
// ---------------------------------------------------------------------------

/// 轻量渠道（Bark/邮件/pushplus 等）默认标题/正文模板，{{变量}} 占位
const DEFAULT_SMS_TITLE_TEMPLATE: &str = "📱短信";
const DEFAULT_SMS_BODY_TEMPLATE: &str = "来自：{{phone_number}}\n内容：{{content}}\n\n本机号码：{{device_name}}\n时间：{{local_time}}";
const DEFAULT_CALL_TITLE_TEMPLATE: &str = "📞来电提醒";
const DEFAULT_CALL_BODY_TEMPLATE: &str = "来电号码：{{phone_number}}\n时长：{{duration}}秒\n\n本机号码：{{device_name}}\n时间：{{local_time}}";

const DEFAULT_DINGTALK_TEMPLATE: &str = r#"{"msgtype":"text","text":{"content":"📱短信\n来自：{{phone_number}}\n内容：{{content}}\n\n本机号码：{{device_name}}\n时间：{{local_time}}"}}"#;

const DEFAULT_FEISHU_TEMPLATE: &str = r#"{"msg_type":"text","content":{"text":"📱短信\n来自：{{phone_number}}\n内容：{{content}}\n\n本机号码：{{device_name}}\n时间：{{local_time}}"}}"#;

const DEFAULT_WECOM_TEMPLATE: &str = r#"{"msgtype":"text","content":{"content":"📱短信\n来自：{{phone_number}}\n内容：{{content}}\n\n本机号码：{{device_name}}\n时间：{{local_time}}"}}"#;

const DEFAULT_DINGTALK_CALL_TEMPLATE: &str = r#"{"msgtype":"text","text":{"content":"📞来电提醒\n来电号码：{{phone_number}}\n时长：{{duration}}秒\n\n本机号码：{{device_name}}\n时间：{{local_time}}"}}"#;

const DEFAULT_FEISHU_CALL_TEMPLATE: &str = r#"{"msg_type":"text","content":{"text":"📞来电提醒\n来电号码：{{phone_number}}\n时长：{{duration}}秒\n\n本机号码：{{device_name}}\n时间：{{local_time}}"}}"#;

const DEFAULT_WECOM_CALL_TEMPLATE: &str = r#"{"msgtype":"text","content":{"content":"📞来电提醒\n来电号码：{{phone_number}}\n时长：{{duration}}秒\n\n本机号码：{{device_name}}\n时间：{{local_time}}"}}"#;

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

    /// 获取设备标识（优先级：自定义 device_name > SIM 本机号码 > "未知设备"）
    fn get_device_label(&self) -> String {
        let device_name = self.config_manager.get_device_name();
        if !device_name.is_empty() {
            return device_name;
        }
        let number = self.self_number.read().unwrap().clone();
        if !number.is_empty() {
            return number;
        }
        "未知设备".to_string()
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
        
        let device_label = self.get_device_label();
        let payload = render_sms_for_channel(&config, message, &device_label);
        self.send_by_channel(config.channel, &config, &payload).await
    }
    
    /// 转发通话记录
    pub async fn forward_call(&self, call: &CallRecord) -> Result<(), String> {
        let config = self.get_config();
        
        if !config.is_channel_enabled() || !config.forward_calls {
            return Ok(());
        }
        
        let device_label = self.get_device_label();
        let payload = render_call_for_channel(&config, call, &device_label);
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
            ChannelType::Pushplus => self.send_push_provider(&config.pushplus, ChannelType::Pushplus, payload).await,
            ChannelType::Serverchan => self.send_push_provider(&config.serverchan, ChannelType::Serverchan, payload).await,
            ChannelType::Pushdeer => self.send_push_provider(&config.pushdeer, ChannelType::Pushdeer, payload).await,
            ChannelType::Ntfy => self.send_push_provider(&config.ntfy, ChannelType::Ntfy, payload).await,
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
        
        let device_label = self.get_device_label();
        let payload = render_sms_for_channel(&config, &test_message, &device_label);
        self.send_by_channel(config.channel, &config, &payload).await?;
        
        Ok(format!("Test message sent via {:?} successfully", config.channel))
    }
    
    // ---------------------------------------------------------------------------
    // 通用推送服务（PushPlus / Server酱 / PushDeer / ntfy，吸收自上游短信推送体系）
    // ---------------------------------------------------------------------------

    /// 发送通用推送服务消息。
    /// payload 沿用 Bark 的 "标题\n\n正文" 约定；服务地址留空用官方默认端点。
    async fn send_push_provider(
        &self,
        cfg: &PushProviderConfig,
        channel: ChannelType,
        payload: &str,
    ) -> Result<(), String> {
        let credential = cfg.credential.trim();
        let topic = cfg.topic.trim();

        // 配置校验（与上游 validate_config 语义一致）
        match channel {
            ChannelType::Ntfy => {
                if topic.is_empty() {
                    return Err("ntfy 主题不能为空".to_string());
                }
            }
            _ => {
                if credential.is_empty() {
                    return Err("当前推送服务缺少凭证".to_string());
                }
            }
        }

        // payload 拆分为 (title, body)，与 Bark 一致
        let (title, body) = if let Some(pos) = payload.find("\n\n") {
            (payload[..pos].to_string(), payload[pos + 2..].to_string())
        } else {
            ("CPE 通知".to_string(), payload.to_string())
        };

        let request = match channel {
            ChannelType::Pushplus => {
                let endpoint = if cfg.url.trim().is_empty() { "https://www.pushplus.plus/send" } else { cfg.url.trim() };
                let mut req_payload = json!({
                    "token": credential,
                    "title": title,
                    "content": body,
                    "template": "markdown",
                });
                if !topic.is_empty() {
                    req_payload["topic"] = json!(topic);
                }
                self.client.post(endpoint).json(&req_payload)
            }
            ChannelType::Serverchan => {
                let base = if cfg.url.trim().is_empty() { "https://sctapi.ftqq.com".to_string() } else { cfg.url.trim().trim_end_matches('/').to_string() };
                let endpoint = format!("{}/{}.send", base, credential);
                self.client.post(endpoint).form(&[
                    ("title", title.as_str()),
                    ("text", title.as_str()),
                    ("desp", body.as_str()),
                ])
            }
            ChannelType::Pushdeer => {
                let endpoint = if cfg.url.trim().is_empty() { "https://api2.pushdeer.com/message/push" } else { cfg.url.trim() };
                self.client.post(endpoint).form(&[
                    ("pushkey", credential),
                    ("text", title.as_str()),
                    ("desp", body.as_str()),
                    ("type", "markdown"),
                ])
            }
            ChannelType::Ntfy => {
                let base = if cfg.url.trim().is_empty() { "https://ntfy.sh".to_string() } else { cfg.url.trim().trim_end_matches('/').to_string() };
                let endpoint = format!("{}/", base);
                let mut request = self.client.post(endpoint).json(&json!({
                    "topic": topic,
                    "title": title,
                    "message": body,
                    "markdown": true,
                }));
                if !credential.is_empty() {
                    request = request.bearer_auth(credential);
                }
                request
            }
            _ => return Err(format!("Unsupported push provider: {:?}", channel)),
        };

        let response = request
            .send()
            .await
            .map_err(|e| format!("Failed to send push message: {}", e))?;

        let status = response.status();
        let response_body = response.text().await.unwrap_or_default();
        check_push_provider_response(channel, status, &response_body)?;
        Ok(())
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
fn render_sms_for_channel(config: &NotificationChannel, sms: &SmsMessage, device_label: &str) -> String {
    let local_time = utc_to_local(&sms.timestamp);
    match config.channel {
        ChannelType::Dingtalk => {
            let tpl = if config.dingtalk.template.is_empty() { DEFAULT_DINGTALK_TEMPLATE } else { &config.dingtalk.template };
            render_template(tpl, sms, None, device_label, &local_time, true)
        }
        ChannelType::Feishu => {
            let tpl = if config.feishu.template.is_empty() { DEFAULT_FEISHU_TEMPLATE } else { &config.feishu.template };
            render_template(tpl, sms, None, device_label, &local_time, true)
        }
        ChannelType::Wecom => {
            let tpl = if config.wecom.template.is_empty() { DEFAULT_WECOM_TEMPLATE } else { &config.wecom.template };
            render_template(tpl, sms, None, device_label, &local_time, true)
        }
        // 轻量渠道：标题/正文均可自定义模板（"标题\n\n正文" 约定，由发送方拆分）
        ChannelType::Email | ChannelType::Bark
        | ChannelType::Pushplus | ChannelType::Serverchan
        | ChannelType::Pushdeer | ChannelType::Ntfy => {
            let title_tpl = if config.sms_title_template.is_empty() { DEFAULT_SMS_TITLE_TEMPLATE } else { &config.sms_title_template };
            let body_tpl = if config.sms_body_template.is_empty() { DEFAULT_SMS_BODY_TEMPLATE } else { &config.sms_body_template };
            let title = render_template(title_tpl, sms, None, device_label, &local_time, false);
            let body = render_template(body_tpl, sms, None, device_label, &local_time, false);
            format!("{}\n\n{}", title, body)
        }
        ChannelType::None => String::new(),
    }
}

/// 根据渠道渲染通话内容
fn render_call_for_channel(config: &NotificationChannel, call: &CallRecord, device_label: &str) -> String {
    let local_time = utc_to_local(&call.start_time);
    
    // 构建一个空 SMS 用于复用 render_template 的通话变量替换
    let dummy_sms = SmsMessage {
        id: 0, direction: String::new(), phone_number: String::new(),
        content: String::new(), timestamp: String::new(), status: String::new(), pdu: None,
    };
    
    match config.channel {
        ChannelType::Dingtalk => {
            let tpl = if config.dingtalk.template.is_empty() { DEFAULT_DINGTALK_CALL_TEMPLATE } else { &config.dingtalk.template };
            render_template(tpl, &dummy_sms, Some(call), device_label, &local_time, true)
        }
        ChannelType::Feishu => {
            let tpl = if config.feishu.template.is_empty() { DEFAULT_FEISHU_CALL_TEMPLATE } else { &config.feishu.template };
            render_template(tpl, &dummy_sms, Some(call), device_label, &local_time, true)
        }
        ChannelType::Wecom => {
            let tpl = if config.wecom.template.is_empty() { DEFAULT_WECOM_CALL_TEMPLATE } else { &config.wecom.template };
            render_template(tpl, &dummy_sms, Some(call), device_label, &local_time, true)
        }
        // 轻量渠道：标题/正文均可自定义模板
        ChannelType::Email | ChannelType::Bark
        | ChannelType::Pushplus | ChannelType::Serverchan
        | ChannelType::Pushdeer | ChannelType::Ntfy => {
            let title_tpl = if config.call_title_template.is_empty() { DEFAULT_CALL_TITLE_TEMPLATE } else { &config.call_title_template };
            let body_tpl = if config.call_body_template.is_empty() { DEFAULT_CALL_BODY_TEMPLATE } else { &config.call_body_template };
            let title = render_template(title_tpl, &dummy_sms, Some(call), device_label, &local_time, false);
            let body = render_template(body_tpl, &dummy_sms, Some(call), device_label, &local_time, false);
            format!("{}\n\n{}", title, body)
        }
        ChannelType::None => String::new(),
    }
}

/// 通用模板替换，支持 {{变量名}} 格式。
/// `json_escape=true` 时 {{content}}/{{message}} 会转义为 JSON 字符串字面量
/// （用于整包 JSON payload 模板）；纯文本模板传 false。
fn render_template(template: &str, sms: &SmsMessage, call: Option<&CallRecord>, device_label: &str, local_time: &str, json_escape: bool) -> String {
    let mut result = template.to_string();

    // 设备标识（新变量 device_name + 兼容旧的 self_number）
    result = result.replace("{{device_name}}", device_label);
    result = result.replace("{{self_number}}", device_label);
    result = result.replace("{{local_time}}", local_time);

    if let Some(c) = call {
        // ========== 通话模板 ==========
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
    } else {
        // ========== 短信模板 ==========
        result = result.replace("{{phone_number}}", &sms.phone_number);
        let content = if json_escape { escape_json_string(&sms.content) } else { sms.content.clone() };
        result = result.replace("{{content}}", &content);
        result = result.replace("{{timestamp}}", &sms.timestamp);
        result = result.replace("{{direction}}", &sms.direction);
        let direction_cn = if sms.direction == "incoming" { "来电" } else if sms.direction == "outgoing" { "去电" } else { &sms.direction };
        result = result.replace("{{direction_cn}}", direction_cn);
        result = result.replace("{{status}}", &sms.status);
        result = result.replace("{{id}}", &sms.id.to_string());
        result = result.replace("{{sender}}", &sms.phone_number);
        result = result.replace("{{message}}", &content);
        result = result.replace("{{time}}", &sms.timestamp);
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

/// 校验通用推送服务的响应（移植自上游 validate_provider_response 语义）。
/// 各服务 HTTP 200 之外还有业务码：pushplus/bark 要求 code=200，
/// serverchan/pushdeer 要求 code=0 或 200，ntfy 只看 HTTP 状态。
fn check_push_provider_response(
    channel: ChannelType,
    status: reqwest::StatusCode,
    body: &str,
) -> Result<(), String> {
    if !status.is_success() {
        let preview: String = body.chars().take(200).collect();
        return Err(format!("推送服务返回错误状态 {} {}", status, preview));
    }

    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(());
    }

    let value = match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };

    let code = value.get("code").and_then(Value::as_i64);
    match channel {
        ChannelType::Pushplus => {
            if let Some(code) = code {
                if code != 200 {
                    return Err(extract_push_provider_error(&value));
                }
            }
        }
        ChannelType::Serverchan | ChannelType::Pushdeer => {
            if let Some(code) = code {
                if code != 0 && code != 200 {
                    return Err(extract_push_provider_error(&value));
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn extract_push_provider_error(value: &Value) -> String {
    let message = value
        .get("msg")
        .or_else(|| value.get("message"))
        .or_else(|| value.get("error"))
        .and_then(Value::as_str)
        .unwrap_or("未知错误");

    format!("推送服务返回失败: {}", message)
}
