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
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tracing::{info, warn};

/// 开机自启动脚本常量（上游 InitScript 功能）
const DEFAULT_LOADER_SCRIPT: &str = r#"#!/bin/sh
/home/root/ttyd/start.sh &
/home/root/udx710 -p 80 &
"#;
const LOADER_SCRIPT_PATH: &str = "/home/root/loader.sh";
const INIT_SCRIPT_PATH: &str = "/home/root/init.sh";
const INIT_SCRIPT_LOADER_COMMAND: &str = "sh /home/root/init.sh &";

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
    /// PushPlus（吸收自上游短信推送体系）
    Pushplus,
    /// Server酱 Turbo
    Serverchan,
    /// PushDeer
    Pushdeer,
    /// ntfy
    Ntfy,
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

/// 通用推送服务配置（pushplus / serverchan / pushdeer / ntfy 共用）
///
/// 吸收自上游短信推送体系的 SmsPushConfig，简化为统一三要素：
/// - credential: 鉴权凭证（token / SendKey / pushkey / 访问令牌）
/// - url: 服务地址（留空用官方默认端点，自建服务填自定义地址）
/// - topic: 主题/分组（ntfy 必填，其余可选）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PushProviderConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub credential: String,
    #[serde(default)]
    pub topic: String,
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
    #[serde(default)]
    pub pushplus: PushProviderConfig,
    #[serde(default)]
    pub serverchan: PushProviderConfig,
    #[serde(default)]
    pub pushdeer: PushProviderConfig,
    #[serde(default)]
    pub ntfy: PushProviderConfig,
    /// 轻量渠道（Bark/邮件/pushplus/serverchan/pushdeer/ntfy）自定义模板，
    /// 支持 {{变量}} 占位；留空使用内置默认格式
    #[serde(default)]
    pub sms_title_template: String,
    #[serde(default)]
    pub sms_body_template: String,
    #[serde(default)]
    pub call_title_template: String,
    #[serde(default)]
    pub call_body_template: String,
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
            pushplus: PushProviderConfig::default(),
            serverchan: PushProviderConfig::default(),
            pushdeer: PushProviderConfig::default(),
            ntfy: PushProviderConfig::default(),
            sms_title_template: String::new(),
            sms_body_template: String::new(),
            call_title_template: String::new(),
            call_body_template: String::new(),
            forward_sms: true,
            forward_calls: true,
        }
    }
}

/// 定时重启配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledRebootConfig {
    /// 是否启用定时重启
    #[serde(default)]
    pub enabled: bool,
    /// 重启间隔天数（1=每天，2=每2天，以此类推）
    #[serde(default = "default_interval_days")]
    pub interval_days: u32,
    /// 重启时间 - 小时（0-23）
    #[serde(default)]
    pub hour: u8,
    /// 重启时间 - 分钟（0-59）
    #[serde(default)]
    pub minute: u8,
}

fn default_interval_days() -> u32 { 1 }

impl Default for ScheduledRebootConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_days: 1,
            hour: 4,
            minute: 0,
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
            ChannelType::Pushplus => {
                if !self.pushplus.credential.is_empty() {
                    Some((ChannelType::Pushplus, &self.pushplus.url))
                } else {
                    None
                }
            }
            ChannelType::Serverchan => {
                if !self.serverchan.credential.is_empty() {
                    Some((ChannelType::Serverchan, &self.serverchan.url))
                } else {
                    None
                }
            }
            ChannelType::Pushdeer => {
                if !self.pushdeer.credential.is_empty() {
                    Some((ChannelType::Pushdeer, &self.pushdeer.url))
                } else {
                    None
                }
            }
            ChannelType::Ntfy => {
                if !self.ntfy.topic.is_empty() {
                    Some((ChannelType::Ntfy, &self.ntfy.url))
                } else {
                    None
                }
            }
        }
    }
}

/// 数据连接配置（流量限额）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataConnectionConfig {
    /// 流量限额（GB）。0 表示不限制。
    #[serde(default)]
    pub limit_gb: f64,
    /// 到达限额后是否自动关闭数据连接
    #[serde(default)]
    pub auto_disable: bool,
    /// 流量自动清零日（每月几号，1-31）。到达该日期自动清零统计并开始新计费周期；
    /// 选 1 即每月 1 号清零（等同正常手机卡月底/月初清零）。非法值（0 或 >31）落库时归为 1。
    #[serde(default = "default_reset_day")]
    pub reset_day: u8,
}

fn default_reset_day() -> u8 { 1 }

impl Default for DataConnectionConfig {
    fn default() -> Self {
        Self {
            limit_gb: 0.0,
            auto_disable: false,
            reset_day: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshConfig {
    #[serde(default = "default_refresh_interval_ms")]
    pub interval_ms: u64,
}

fn default_refresh_interval_ms() -> u64 {
    5_000
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            interval_ms: default_refresh_interval_ms(),
        }
    }
}

impl RefreshConfig {
    pub fn sanitize(mut self) -> Self {
        self.interval_ms = sanitize_refresh_interval_ms(self.interval_ms);
        self
    }

    pub fn heartbeat_timeout_ms(&self) -> u64 {
        let base = self.interval_ms.max(1_000);
        if self.interval_ms == 0 {
            30_000
        } else {
            (base.saturating_mul(4)).clamp(15_000, 120_000)
        }
    }

    pub fn active_watchdog_interval_ms(&self) -> u64 {
        if self.interval_ms == 0 {
            15_000
        } else {
            self.interval_ms.max(5_000)
        }
    }

    pub fn idle_watchdog_interval_ms(&self) -> u64 {
        self.active_watchdog_interval_ms()
            .saturating_mul(6)
            .max(60_000)
    }
}

fn sanitize_refresh_interval_ms(interval_ms: u64) -> u64 {
    match interval_ms {
        0 => 0,
        1..=999 => 1_000,
        value => value.min(60_000),
    }
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub webhook: NotificationChannel,
    /// 设备自定义名称（用于推送消息标识，如"客厅CPE"）
    #[serde(default)]
    pub device_name: String,
    /// 定时重启配置
    #[serde(default)]
    pub scheduled_reboot: ScheduledRebootConfig,
    /// 数据连接配置（流量限额）
    #[serde(default)]
    pub data_connection: DataConnectionConfig,
    /// 前端刷新间隔配置（上游）
    #[serde(default)]
    pub refresh: RefreshConfig,
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
                        Ok(cfg) => AppConfig {
                            refresh: cfg.refresh.sanitize(),
                            ..cfg
                        },
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

    /// 获取设备名称
    pub fn get_device_name(&self) -> String {
        self.config.read().unwrap().device_name.clone()
    }

    /// 设置设备名称
    pub fn set_device_name(&self, name: &str) -> Result<(), String> {
        {
            let mut config = self.config.write().unwrap();
            config.device_name = name.to_string();
        }
        self.save()
    }

    /// 获取定时重启配置
    pub fn get_scheduled_reboot(&self) -> ScheduledRebootConfig {
        self.config.read().unwrap().scheduled_reboot.clone()
    }

    /// 设置定时重启配置
    pub fn set_scheduled_reboot(&self, cfg: ScheduledRebootConfig) -> Result<(), String> {
        {
            let mut config = self.config.write().unwrap();
            config.scheduled_reboot = cfg;
        }
        self.save()
    }

    /// 获取数据连接配置（流量限额）
    pub fn get_data_connection_config(&self) -> DataConnectionConfig {
        self.config.read().unwrap().data_connection.clone()
    }

    /// 设置数据连接配置（流量限额）
    pub fn set_data_connection_config(&self, mut cfg: DataConnectionConfig) -> Result<(), String> {
        // 归一化清零日：0 或 >31 视为 1（每月 1 号清零，等同正常手机卡）
        if cfg.reset_day == 0 || cfg.reset_day > 31 {
            cfg.reset_day = 1;
        }
        {
            let mut config = self.config.write().unwrap();
            config.data_connection = cfg;
        }
        self.save()
    }

    /// 获取前端刷新配置
    pub fn get_refresh(&self) -> RefreshConfig {
        self.config.read().unwrap().refresh.clone().sanitize()
    }

    /// 设置前端刷新配置
    pub fn set_refresh(&self, refresh: RefreshConfig) -> Result<(), String> {
        {
            let mut config = self.config.write().unwrap();
            config.refresh = refresh.sanitize();
        }
        self.save()
    }

    #[allow(dead_code)]
    pub fn set(&self, config: AppConfig) -> Result<(), String> {
        {
            let mut current = self.config.write().unwrap();
            *current = AppConfig {
                refresh: config.refresh.sanitize(),
                ..config
            };
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
            *config = AppConfig {
                refresh: new_config.refresh.sanitize(),
                ..new_config
            };
        }
        
        Ok(())
    }
}

/// 获取默认配置文件路径
pub fn get_persistent_root_dir() -> PathBuf {
    let device_root = PathBuf::from("/data");
    if device_root.exists() {
        return device_root;
    }

    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn get_default_config_path() -> PathBuf {
    get_persistent_root_dir().join("config.json")
}

fn normalize_newlines(content: &str) -> String {
    content.replace("\r\n", "\n")
}

fn is_ota_hook_line(line: &str) -> bool {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }

    trimmed == "sh /home/root/ota.sh &"
        || trimmed == "/home/root/ota.sh"
        || trimmed == "/home/root/ota.sh &"
        || trimmed.starts_with("sh /home/root/ota.sh")
}

fn is_init_hook_line(line: &str) -> bool {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with('#') {
        return false;
    }

    trimmed == INIT_SCRIPT_LOADER_COMMAND
        || trimmed == INIT_SCRIPT_PATH
        || trimmed == format!("{} &", INIT_SCRIPT_PATH)
        || trimmed.starts_with(&format!("sh {}", INIT_SCRIPT_PATH))
}

fn loader_contains_ota_command(content: &str) -> bool {
    content.lines().any(is_ota_hook_line)
}

fn loader_contains_init_command(content: &str) -> bool {
    content.lines().any(is_init_hook_line)
}

fn remove_ota_command_from_loader(content: &str) -> String {
    let normalized = normalize_newlines(content);
    let mut filtered_lines: Vec<&str> = normalized
        .lines()
        .filter(|line| !is_ota_hook_line(line))
        .collect();

    while filtered_lines.last().is_some_and(|line| line.trim().is_empty()) {
        filtered_lines.pop();
    }

    if filtered_lines.is_empty() {
        return String::new();
    }

    format!("{}\n", filtered_lines.join("\n"))
}

fn append_init_command_to_loader(content: &str) -> String {
    let normalized = normalize_newlines(content);

    if loader_contains_init_command(&normalized) {
        return format!("{}\n", normalized.trim_end_matches('\n'));
    }

    let base = if normalized.trim().is_empty() {
        DEFAULT_LOADER_SCRIPT.trim_end_matches('\n').to_string()
    } else {
        normalized.trim_end_matches('\n').to_string()
    };

    format!("{}\n{}\n", base, INIT_SCRIPT_LOADER_COMMAND)
}

fn loader_uses_ab_bootstrap(content: &str) -> bool {
    content.contains("UDX710 OTA bootstrap")
        || content.contains("OTA_STATE_FILE=\"/home/root/ota/state.env\"")
}

fn loader_is_plain_legacy_bootstrap(content: &str) -> bool {
    let script_lines: Vec<&str> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with('#') || *line == "#!/bin/sh")
        .collect();

    if script_lines.len() < 3 {
        return false;
    }

    if script_lines[0] != "#!/bin/sh" {
        return false;
    }

    if script_lines[1] != "/home/root/ttyd/start.sh &"
        || script_lines[2] != "/home/root/udx710 -p 80 &"
    {
        return false;
    }

    script_lines[3..]
        .iter()
        .all(|line| *line == INIT_SCRIPT_LOADER_COMMAND)
}

fn set_executable_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)
            .map_err(|e| format!("Failed to read metadata for {}: {}", path.display(), e))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .map_err(|e| format!("Failed to set permissions for {}: {}", path.display(), e))?;
    }

    Ok(())
}

pub fn ensure_loader_hooks_init() -> Result<(), String> {
    let loader_path = PathBuf::from(LOADER_SCRIPT_PATH);
    let current_content = if loader_path.exists() {
        fs::read_to_string(&loader_path)
            .map_err(|e| format!("Failed to read loader.sh: {}", e))?
    } else {
        String::new()
    };

    let stripped_content = remove_ota_command_from_loader(&current_content);
    let missing_backend_command = !stripped_content
        .lines()
        .any(|line| line.trim() == "/home/root/udx710 -p 80 &");

    let base_content = if loader_uses_ab_bootstrap(&current_content)
        || loader_contains_ota_command(&current_content)
        || missing_backend_command
    {
        DEFAULT_LOADER_SCRIPT.to_string()
    } else if current_content.trim().is_empty()
        || loader_is_plain_legacy_bootstrap(&current_content)
    {
        DEFAULT_LOADER_SCRIPT.to_string()
    } else {
        stripped_content
    };

    let updated_content = append_init_command_to_loader(&base_content);

    if let Some(parent) = loader_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create loader.sh directory: {}", e))?;
    }

    fs::write(&loader_path, updated_content)
        .map_err(|e| format!("Failed to write loader.sh: {}", e))?;
    set_executable_permissions(&loader_path)?;

    let _ = fs::remove_file("/home/root/ota.sh");

    Ok(())
}

pub fn get_init_script() -> Result<crate::models::InitScriptResponse, String> {
    let loader_content = if Path::new(LOADER_SCRIPT_PATH).exists() {
        fs::read_to_string(LOADER_SCRIPT_PATH)
            .map_err(|e| format!("Failed to read loader.sh: {}", e))?
    } else {
        DEFAULT_LOADER_SCRIPT.to_string()
    };

    let script = match fs::read_to_string(INIT_SCRIPT_PATH) {
        Ok(content) => normalize_newlines(&content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Failed to read init.sh: {}", e)),
    };

    Ok(crate::models::InitScriptResponse {
        script,
        init_path: INIT_SCRIPT_PATH.to_string(),
        loader_path: LOADER_SCRIPT_PATH.to_string(),
        loader_hooked: loader_contains_init_command(&loader_content),
    })
}

pub fn set_init_script(script: String) -> Result<crate::models::InitScriptResponse, String> {
    let init_path = PathBuf::from(INIT_SCRIPT_PATH);
    if let Some(parent) = init_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create init.sh directory: {}", e))?;
    }

    fs::write(&init_path, normalize_newlines(&script))
        .map_err(|e| format!("Failed to write init.sh: {}", e))?;
    set_executable_permissions(&init_path)?;

    ensure_loader_hooks_init()?;

    get_init_script()
}

#[cfg(test)]
mod tests {
    use super::{
        append_init_command_to_loader,
        loader_contains_init_command,
        loader_contains_ota_command,
        remove_ota_command_from_loader,
        INIT_SCRIPT_LOADER_COMMAND,
    };

    #[test]
    fn append_init_command_once_for_new_loader() {
        let loader = "#!/bin/sh\n/home/root/ttyd/start.sh &\n/home/root/udx710 -p 80 &\n";
        let updated = append_init_command_to_loader(loader);

        assert!(updated.contains(INIT_SCRIPT_LOADER_COMMAND));
        assert_eq!(updated.matches(INIT_SCRIPT_LOADER_COMMAND).count(), 1);
    }

    #[test]
    fn append_init_command_is_idempotent() {
        let loader = format!(
            "#!/bin/sh\n/home/root/ttyd/start.sh &\n/home/root/udx710 -p 80 &\n{}\n",
            INIT_SCRIPT_LOADER_COMMAND
        );
        let updated = append_init_command_to_loader(&loader);

        assert_eq!(updated.matches(INIT_SCRIPT_LOADER_COMMAND).count(), 1);
    }

    #[test]
    fn loader_detects_init_command() {
        let loader = format!("#!/bin/sh\n{}\n", INIT_SCRIPT_LOADER_COMMAND);
        assert!(loader_contains_init_command(&loader));
    }

    #[test]
    fn loader_ignores_commented_init_command() {
        let loader = format!("#!/bin/sh\n# {}\n", INIT_SCRIPT_LOADER_COMMAND);
        assert!(!loader_contains_init_command(&loader));
    }

    #[test]
    fn remove_ota_command_from_loader_strips_old_hook() {
        let loader = "#!/bin/sh\n/home/root/ttyd/start.sh &\nsh /home/root/ota.sh &\n/home/root/udx710 -p 80 &\n";
        let updated = remove_ota_command_from_loader(loader);

        assert!(!loader_contains_ota_command(&updated));
        assert!(updated.contains("/home/root/udx710 -p 80 &"));
    }
}
