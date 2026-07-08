/*
 * @Author: 1orz cloudorzi@gmail.com
 * @Date: 2025-12-09 17:34:01
 * @LastEditors: 1orz cloudorzi@gmail.com
 * @LastEditTime: 2025-12-13 12:45:58
 * @FilePath: /udx710-backend/backend/src/config.rs
 * @Description: 
 * 
 * Copyright (c) 2025 by 1orz, All Rights Reserved. 
 */
//! 配置管理模块
//!
//! 使用 JSON 文件存储用户配置，支持热更新

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

/// 通知渠道类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChannelType {
    #[default]
    None,
    Dingtalk,
    Feishu,
    Wecom,
    Email,
    Bark,
}

/// 钉钉机器人配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DingtalkConfig {
    pub url: String,
    pub secret: String,
    #[serde(default)]
    pub template: String,
}

/// 飞书机器人配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeishuConfig {
    pub url: String,
    #[serde(default)]
    pub secret: String,
    #[serde(default)]
    pub template: String,
}

/// 企业微信机器人配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WecomConfig {
    pub url: String,
    #[serde(default)]
    pub secret: String,
    #[serde(default)]
    pub template: String,
}

/// 邮件配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub use_tls: bool,
    pub username: String,
    pub password: String,
    pub from_name: String,
    pub to_addresses: String,
    #[serde(default)]
    pub subject_prefix: String,
}

/// Bark 推送配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BarkConfig {
    pub server_url: String,
    pub device_key: String,
    #[serde(default)]
    pub sound: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub group: String,
}

/// 通知渠道配置（互斥单选）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationChannel {
    pub channel: ChannelType,
    /// 各渠道配置字段，全部可选，激活哪个填哪个
    #[serde(default)]
    pub dingtalk: DingtalkConfig,
    #[serde(default)]
    pub feishu: FeishuConfig,
    #[serde(default)]
    pub wecom: WecomConfig,
    #[serde(default)]
    pub email: EmailConfig,
    #[serde(default)]
    pub bark: BarkConfig,
    /// 全局开关
    pub forward_sms: bool,
    pub forward_calls: bool,
}

impl Default for NotificationChannel {
    fn default() -> Self {
        Self {
            channel: ChannelType::None,
            dingtalk: DingtalkConfig::default(),
            feishu: FeishuConfig::default(),
            wecom: WecomConfig::default(),
            email: EmailConfig::default(),
            bark: BarkConfig::default(),
            forward_sms: true,
            forward_calls: true,
        }
    }
}

impl NotificationChannel {
    /// 判断通知渠道是否启用
    pub fn is_channel_enabled(&self) -> bool {
        self.channel != ChannelType::None
    }

    /// 获取当前激活渠道的 URL
    /// 返回 (ChannelType, &str)
    pub fn get_active_url(&self) -> Option<(ChannelType, &str)> {
        match self.channel {
            ChannelType::None => None,
            ChannelType::Dingtalk => {
                if !self.dingtalk.url.is_empty() {
                    Some((ChannelType::Dingtalk, &self.dingtalk.url))
                } else {
                    None
                }
            }
            ChannelType::Feishu => {
                if !self.feishu.url.is_empty() {
                    Some((ChannelType::Feishu, &self.feishu.url))
                } else {
                    None
                }
            }
            ChannelType::Wecom => {
                if !self.wecom.url.is_empty() {
                    Some((ChannelType::Wecom, &self.wecom.url))
                } else {
                    None
                }
            }
            ChannelType::Email => None,
            ChannelType::Bark => {
                if !self.bark.device_key.is_empty() {
                    Some((ChannelType::Bark, &self.bark.device_key))
                } else {
                    None
                }
            }
        }
    }
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub webhook: NotificationChannel,
}

/// 配置管理器
pub struct ConfigManager {
    config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
}

impl ConfigManager {
    /// 创建新的配置管理器
    pub fn new(config_path: PathBuf) -> Self {
        let config = if config_path.exists() {
            match fs::read_to_string(&config_path) {
                Ok(content) => {
                    match serde_json::from_str::<AppConfig>(&content) {
                        Ok(cfg) => cfg,
                        Err(e) => {
                            warn!(error = %e, "Failed to parse config file, using defaults");
                            AppConfig::default()
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to read config file, using defaults");
                    AppConfig::default()
                }
            }
        } else {
            info!("No config file found, using defaults");
            AppConfig::default()
        };

        let manager = Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
        };
        
        // 保存默认配置（如果文件不存在）
        if !manager.config_path.exists() {
            let _ = manager.save();
        }
        
        manager
    }
    
    /// 获取当前配置
    #[allow(dead_code)]
    pub fn get(&self) -> AppConfig {
        self.config.read().unwrap().clone()
    }
    
    /// 获取 Webhook（通知渠道）配置
    pub fn get_webhook(&self) -> NotificationChannel {
        self.config.read().unwrap().webhook.clone()
    }
    
    /// 更新 Webhook（通知渠道）配置
    pub fn set_webhook(&self, webhook: NotificationChannel) -> Result<(), String> {
        {
            let mut config = self.config.write().unwrap();
            config.webhook = webhook;
        }
        self.save()
    }
    
    /// 更新整个配置
    #[allow(dead_code)]
    pub fn set(&self, config: AppConfig) -> Result<(), String> {
        {
            let mut current = self.config.write().unwrap();
            *current = config;
        }
        self.save()
    }
    
    /// 保存配置到文件
    pub fn save(&self) -> Result<(), String> {
        let config = self.config.read().unwrap();
        let content = serde_json::to_string_pretty(&*config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        
        // 确保目录存在
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }
        
        fs::write(&self.config_path, content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;
        
        Ok(())
    }
    
    /// 重新加载配置
    #[allow(dead_code)]
    pub fn reload(&self) -> Result<(), String> {
        if !self.config_path.exists() {
            return Err("Config file does not exist".to_string());
        }
        
        let content = fs::read_to_string(&self.config_path)
            .map_err(|e| format!("Failed to read config file: {}", e))?;
        
        let new_config: AppConfig = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse config file: {}", e))?;
        
        {
            let mut config = self.config.write().unwrap();
            *config = new_config;
        }
        
        Ok(())
    }
}

/// 获取默认配置文件路径
pub fn get_default_config_path() -> PathBuf {
    // 尝试 /data/config.json（设备上的持久化目录）
    let device_path = PathBuf::from("/data/config.json");
    if device_path.parent().map(|p| p.exists()).unwrap_or(false) {
        return device_path;
    }
    
    // 回退到当前目录
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("config.json")
}
