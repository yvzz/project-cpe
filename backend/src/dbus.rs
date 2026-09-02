/*
 * @Author: 1orz cloudorzi@gmail.com
 * @Date: 2025-12-10 09:19:05
 * @LastEditors: 1orz cloudorzi@gmail.com
 * @LastEditTime: 2025-12-13 12:46:02
 * @FilePath: /udx710-backend/backend/src/dbus.rs
 * @Description: 
 * 
 * Copyright (c) 2025 by 1orz, All Rights Reserved. 
 */
//! D-Bus 通信模块
//! 
//! 处理与 ofono D-Bus 服务的通信

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn};
use zbus::{proxy, zvariant::OwnedValue, Connection, Proxy};

use crate::config::ConfigManager;
use crate::models::{
    AirplaneModeResponse, ApnContext, DeviceInfoResponse, NetworkInfoResponse, QosInfoResponse, RadioMode,
    RadioModeResponse, ServingCell, SimInfoResponse,
};
use crate::serial::with_serial;
use crate::usage::DataUsageTracker;

/// ofono NetworkMonitor 代理接口
#[proxy(
    interface = "org.ofono.NetworkMonitor",
    default_service = "org.ofono",
    default_path = "/ril_0",
    assume_defaults = true
)]
pub trait NetworkMonitor {
    /// 获取服务小区信息
    fn get_serving_cell_information(
        &self,
    ) -> zbus::Result<HashMap<String, zbus::zvariant::OwnedValue>>;
}

/// ofono ConnectionContext 代理接口
#[proxy(
    interface = "org.ofono.ConnectionContext",
    default_service = "org.ofono",
    default_path = "/ril_0/context2",
    assume_defaults = true
)]
pub trait ConnectionContext {
    /// 获取连接上下文的所有属性
    fn get_properties(&self) -> zbus::Result<HashMap<String, zbus::zvariant::OwnedValue>>;
    
    /// 设置连接上下文的属性
    fn set_property(&self, name: &str, value: zbus::zvariant::Value<'_>) -> zbus::Result<()>;
}

/// ofono SimManager 代理接口
#[proxy(
    interface = "org.ofono.SimManager",
    default_service = "org.ofono",
    default_path = "/ril_0",
    assume_defaults = true
)]
pub trait SimManager {
    /// 获取SIM卡所有属性
    fn get_properties(&self) -> zbus::Result<HashMap<String, zbus::zvariant::OwnedValue>>;
}

/// ofono MessageManager 代理接口
#[proxy(
    interface = "org.ofono.MessageManager",
    default_service = "org.ofono",
    default_path = "/ril_0",
    assume_defaults = true
)]
pub trait MessageManager {
    /// 获取消息管理器所有属性
    fn get_properties(&self) -> zbus::Result<HashMap<String, zbus::zvariant::OwnedValue>>;
}

/// ofono NetworkRegistration 代理接口
#[proxy(
    interface = "org.ofono.NetworkRegistration",
    default_service = "org.ofono",
    default_path = "/ril_0",
    assume_defaults = true
)]
pub trait NetworkRegistration {
    /// 获取网络注册所有属性
    fn get_properties(&self) -> zbus::Result<HashMap<String, zbus::zvariant::OwnedValue>>;
}

/// ofono RadioSettings 代理接口
#[proxy(
    interface = "org.ofono.RadioSettings",
    default_service = "org.ofono",
    default_path = "/ril_0",
    assume_defaults = true
)]
pub trait RadioSettings {
    /// 获取无线设置所有属性
    fn get_properties(&self) -> zbus::Result<HashMap<String, zbus::zvariant::OwnedValue>>;
    
    /// 设置无线设置属性
    fn set_property(&self, name: &str, value: zbus::zvariant::Value<'_>) -> zbus::Result<()>;
}

/// ofono Modem 代理接口
#[proxy(
    interface = "org.ofono.Modem",
    default_service = "org.ofono",
    default_path = "/ril_0",
    assume_defaults = true
)]
pub trait Modem {
    /// 获取调制解调器所有属性
    fn get_properties(&self) -> zbus::Result<HashMap<String, zbus::zvariant::OwnedValue>>;
    
    /// 设置调制解调器属性
    fn set_property(&self, name: &str, value: zbus::zvariant::Value<'_>) -> zbus::Result<()>;
}

/// 通过 D-Bus 发送 AT 指令
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `cmd` - AT 指令字符串
///
/// # Returns
/// AT 指令的响应结果
pub async fn send_at_command(conn: &Connection, cmd: &str) -> zbus::Result<String> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.Modem").await?;
        let result: String = proxy.call("SendAtcmd", &(cmd)).await?;
        Ok(result)
    }).await
}

/// 获取服务小区信息
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// 服务小区信息结构
pub async fn get_serving_cell_info(conn: &Connection) -> zbus::Result<ServingCell> {
    with_serial(async {
        let proxy = NetworkMonitorProxy::new(conn).await?;
        let cell_info: HashMap<String, OwnedValue> = proxy.get_serving_cell_information().await?;

        let tech = cell_info
            .get("Technology")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());

        let cell_id = parse_u32_from_keys(&cell_info, &["NCellId", "CellId", "NRCellID"]);
        let tac = parse_u32_from_keys(&cell_info, &["TrackingAreaCode"]);

        Ok(ServingCell { tech, cell_id, tac })
    }).await
}

/// 查找第一个有效的 internet 类型 context 路径
///
/// 遍历所有 context，返回第一个类型为 internet 且配置了 APN 的 context 路径。
/// 如果没有配置 APN 的 context，则返回第一个 internet 类型的 context。
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// context 路径字符串
pub async fn find_internet_context(conn: &Connection) -> zbus::Result<String> {
    let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.ConnectionManager").await?;
    let contexts: Vec<(zbus::zvariant::OwnedObjectPath, HashMap<String, OwnedValue>)> = 
        proxy.call("GetContexts", &()).await?;
    
    let mut first_internet_context: Option<String> = None;
    
    for (path, props) in contexts {
        let context_type = props
            .get("Type")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default();
        
        if context_type == "internet" {
            let apn = props
                .get("AccessPointName")
                .and_then(|v| String::try_from(v.clone()).ok())
                .unwrap_or_default();
            
            // 如果配置了 APN，优先返回这个 context
            if !apn.is_empty() {
                return Ok(path.to_string());
            }
            
            // 记录第一个 internet 类型的 context
            if first_internet_context.is_none() {
                first_internet_context = Some(path.to_string());
            }
        }
    }
    
    // 返回第一个 internet context，如果没有则返回默认值
    Ok(first_internet_context.unwrap_or_else(|| "/ril_0/context2".to_string()))
}

/// 获取所有 APN Context 列表
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// APN Context 列表
pub async fn get_all_apn_contexts(conn: &Connection) -> zbus::Result<Vec<ApnContext>> {
    let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.ConnectionManager").await?;
    let contexts: Vec<(zbus::zvariant::OwnedObjectPath, HashMap<String, OwnedValue>)> = 
        proxy.call("GetContexts", &()).await?;
    
    let mut result = Vec::new();
    
    for (path, props) in contexts {
        let context_type = props
            .get("Type")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_default();
        
        // 只返回 internet 类型的 context
        if context_type == "internet" {
            let apn_context = ApnContext {
                path: path.to_string(),
                name: props
                    .get("Name")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_else(|| "Internet".to_string()),
                active: props
                    .get("Active")
                    .and_then(|v| bool::try_from(v.clone()).ok())
                    .unwrap_or(false),
                apn: props
                    .get("AccessPointName")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_default(),
                protocol: props
                    .get("Protocol")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_else(|| "ip".to_string()),
                username: props
                    .get("Username")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_default(),
                password: props
                    .get("Password")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_default(),
                auth_method: props
                    .get("AuthenticationMethod")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_else(|| "chap".to_string()),
                context_type,
            };
            result.push(apn_context);
        }
    }
    
    Ok(result)
}

/// 设置 APN 属性
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `context_path` - context 的 D-Bus 路径
/// * `property` - 属性名
/// * `value` - 属性值
///
/// # Returns
/// 操作结果
pub async fn set_apn_property(
    conn: &Connection, 
    context_path: &str, 
    property: &str, 
    value: &str
) -> zbus::Result<()> {
    with_serial(async {
        let proxy = ConnectionContextProxy::builder(conn)
            .path(context_path)?
            .build()
            .await?;
        
        proxy.set_property(property, zbus::zvariant::Value::Str(value.into())).await?;
        Ok(())
    }).await
}

/// 批量设置 APN 属性
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `context_path` - context 的 D-Bus 路径
/// * `apn` - APN 名称（可选）
/// * `protocol` - 协议（可选）
/// * `username` - 用户名（可选）
/// * `password` - 密码（可选）
/// * `auth_method` - 认证方式（可选）
///
/// # Returns
/// 操作结果
pub async fn set_apn_properties(
    conn: &Connection,
    context_path: &str,
    apn: Option<&str>,
    protocol: Option<&str>,
    username: Option<&str>,
    password: Option<&str>,
    auth_method: Option<&str>,
) -> zbus::Result<()> {
    // 先检查 context 是否激活，如果激活需要先关闭
    let proxy = ConnectionContextProxy::builder(conn)
        .path(context_path)?
        .build()
        .await?;
    
    let props = proxy.get_properties().await?;
    let was_active = props
        .get("Active")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    
    // 如果 context 是激活状态，先关闭它
    if was_active {
        with_serial(async {
            let proxy = ConnectionContextProxy::builder(conn)
                .path(context_path)?
                .build()
                .await?;
            proxy.set_property("Active", zbus::zvariant::Value::Bool(false)).await?;
            Ok::<(), zbus::Error>(())
        }).await?;
        
        // 等待一下让状态稳定
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
    
    // 设置各个属性
    if let Some(apn_val) = apn {
        set_apn_property(conn, context_path, "AccessPointName", apn_val).await?;
    }
    
    if let Some(protocol_val) = protocol {
        set_apn_property(conn, context_path, "Protocol", protocol_val).await?;
    }
    
    if let Some(username_val) = username {
        set_apn_property(conn, context_path, "Username", username_val).await?;
    }
    
    if let Some(password_val) = password {
        set_apn_property(conn, context_path, "Password", password_val).await?;
    }
    
    if let Some(auth_method_val) = auth_method {
        set_apn_property(conn, context_path, "AuthenticationMethod", auth_method_val).await?;
    }
    
    // 如果之前是激活状态，重新激活
    if was_active {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        with_serial(async {
            let proxy = ConnectionContextProxy::builder(conn)
                .path(context_path)?
                .build()
                .await?;
            proxy.set_property("Active", zbus::zvariant::Value::Bool(true)).await?;
            Ok::<(), zbus::Error>(())
        }).await?;
    }
    
    Ok(())
}

/// 设置数据连接状态
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `active` - true 开启数据流量，false 关闭数据流量
///
/// # Returns
/// 操作结果
pub async fn set_data_connection(conn: &Connection, active: bool) -> zbus::Result<()> {
    with_serial(async {
        // 自动查找有效的 internet context
        let context_path = find_internet_context(conn).await?;
        
        let proxy = ConnectionContextProxy::builder(conn)
            .path(context_path)?
            .build()
            .await?;
        proxy.set_property("Active", zbus::zvariant::Value::Bool(active)).await?;
        Ok(())
    }).await
}

/// 获取数据连接状态
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// 数据连接是否激活
pub async fn get_data_connection_status(conn: &Connection) -> zbus::Result<bool> {
    // 自动查找有效的 internet context
    let context_path = find_internet_context(conn).await?;
    
    let proxy = ConnectionContextProxy::builder(conn)
        .path(context_path)?
        .build()
        .await?;
    let properties = proxy.get_properties().await?;
    
    let active = properties
        .get("Active")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    
    Ok(active)
}

/// 获取漫游状态
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// (roaming_allowed, is_roaming) 元组
pub async fn get_roaming_status(conn: &Connection) -> zbus::Result<(bool, bool)> {
    // 获取 ConnectionManager 的 RoamingAllowed 属性
    let cm_proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.ConnectionManager").await?;
    let cm_props: std::collections::HashMap<String, OwnedValue> = cm_proxy.call("GetProperties", &()).await?;
    
    let roaming_allowed = cm_props
        .get("RoamingAllowed")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    
    // 获取 NetworkRegistration 的 Status 属性判断是否漫游
    let net_proxy = NetworkRegistrationProxy::new(conn).await?;
    let net_props = net_proxy.get_properties().await?;
    
    let status = net_props
        .get("Status")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "unknown".to_string());
    
    let is_roaming = status == "roaming";
    
    Ok((roaming_allowed, is_roaming))
}

/// 设置漫游开关
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `allowed` - true 允许漫游数据，false 禁止漫游数据
///
/// # Returns
/// 操作结果
pub async fn set_roaming_allowed(conn: &Connection, allowed: bool) -> zbus::Result<()> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.ConnectionManager").await?;
        let value = zbus::zvariant::Value::Bool(allowed);
        proxy.call::<_, _, ()>("SetProperty", &("RoamingAllowed", value)).await?;
        Ok(())
    }).await
}

/// 初始化数据连接（程序启动时调用）
///
/// 检查当前数据连接状态，如果未激活则尝试自动激活。
/// 这个函数会在后台静默执行，不会阻塞服务启动。
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// 初始化结果消息
pub async fn init_data_connection(conn: &Connection) -> String {
    // 1. 先检查网络注册状态
    match NetworkRegistrationProxy::new(conn).await {
        Ok(net_proxy) => {
            if let Ok(props) = net_proxy.get_properties().await {
                let status = props
                    .get("Status")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_else(|| "unknown".to_string());
                
                if status != "registered" && status != "roaming" {
                    return format!("Network not registered (status: {}), skipping data connection", status);
                }
            }
        }
        Err(e) => {
            return format!("Failed to check network status: {}", e);
        }
    }
    
    // 2. 自动查找有效的 internet context
    let context_path = match find_internet_context(conn).await {
        Ok(path) => path,
        Err(e) => {
            return format!("Failed to find internet context: {}", e);
        }
    };
    
    // 3. 获取 context 的属性
    let proxy = match ConnectionContextProxy::builder(conn)
        .path(context_path.as_str())
        .and_then(|b| Ok(b))
    {
        Ok(builder) => match builder.build().await {
            Ok(p) => p,
            Err(e) => return format!("Failed to create context proxy: {}", e),
        },
        Err(e) => return format!("Failed to build context path: {}", e),
    };
    
    let props = match proxy.get_properties().await {
        Ok(p) => p,
        Err(e) => return format!("Failed to get context properties: {}", e),
    };
    
    // 4. 检查是否已激活
    let active = props
        .get("Active")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    
    if active {
        return format!("Data connection already active ({})", context_path);
    }
    
    // 5. 检查 APN 是否配置
    let apn = props
        .get("AccessPointName")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();
    
    if apn.is_empty() {
        return format!("APN not configured on {}, skipping auto-connect", context_path);
    }
    
    // 6. 尝试激活数据连接
    match set_data_connection(conn, true).await {
        Ok(_) => format!("Data connection activated on {} (APN: {})", context_path, apn),
        Err(e) => format!("Failed to activate data connection: {}", e),
    }
}

/// 根据 MCC/MNC 获取推荐的 APN 配置
///
/// # Arguments
/// * `mcc` - 移动国家代码
/// * `mnc` - 移动网络代码
///
/// # Returns
/// (apn, protocol) 元组，如果未找到则返回 None
fn get_recommended_apn(mcc: &str, mnc: &str) -> Option<(&'static str, &'static str)> {
    match (mcc, mnc) {
        // 中国移动 (46000, 46002, 46007, 46008)
        ("460", "00") | ("460", "02") | ("460", "07") | ("460", "08") => Some(("cmnet", "dual")),
        // 中国联通 (46001, 46006, 46009)
        ("460", "01") | ("460", "06") | ("460", "09") => Some(("3gnet", "dual")),
        // 中国电信 (46003, 46005, 46011)
        ("460", "03") | ("460", "05") | ("460", "11") => Some(("ctnet", "dual")),
        // 中国广电 (46015)
        ("460", "15") => Some(("cbnet", "dual")),
        _ => None,
    }
}

/// 读取当前网络注册信息中的 MCC/MNC
///
/// # Returns
/// 成功返回 `(mcc, mnc)`；属性缺失、为空或读取失败时返回 `None`
async fn read_operator_mcc_mnc(conn: &Connection) -> Option<(String, String)> {
    let net_proxy = NetworkRegistrationProxy::new(conn).await.ok()?;

    let props = net_proxy.get_properties().await.ok()?;

    let mcc = props
        .get("MobileCountryCode")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    let mnc = props
        .get("MobileNetworkCode")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    if mcc.is_empty() || mnc.is_empty() {
        return None;
    }

    Some((mcc, mnc))
}

/// 读取当前 SIM 卡的 ICCID
///
/// ICCID 唯一标识一张 SIM 卡，watchdog 用它来判断是否发生了换卡。
///
/// # Returns
/// 读取成功且非空时返回 `Some(iccid)`，否则返回 `None`
async fn read_sim_iccid(conn: &Connection) -> Option<String> {
    let sim_proxy = SimManagerProxy::new(conn).await.ok()?;

    let props = sim_proxy.get_properties().await.ok()?;

    props
        .get("CardIdentifier")
        .and_then(|v| String::try_from(v.clone()).ok())
        .filter(|iccid| !iccid.is_empty())
}

/// 判断 AT 指令响应是否表示失败
///
/// 不同模组的回包格式差异很大，这里采用保守策略：
/// 只有响应为空，或明确包含 ERROR / NO CARRIER 时才判为失败，
/// 其余（含 "OK"、以及模组直接回显指令的情形）都视为成功。
fn at_response_failed(response: &str) -> bool {
    let trimmed = response.trim();

    if trimmed.is_empty() {
        return true;
    }

    let upper = trimmed.to_uppercase();
    upper.contains("ERROR") || upper.contains("NO CARRIER")
}

/// 等待 SIM 卡就绪
///
/// 轮询 `org.ofono.SimManager` 的 `Present` 属性，直到 SIM 被识别。
/// 轮询的睡眠在锁外进行，不会长时间占用全局 D-Bus 串行锁。
async fn wait_for_sim_ready(conn: &Connection, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;

    loop {
        let present = with_serial(async {
            let proxy = SimManagerProxy::new(conn)
                .await
                .map_err(|e| format!("SimManager unavailable: {}", e))?;

            let props = proxy
                .get_properties()
                .await
                .map_err(|e| format!("Failed to read SIM properties: {}", e))?;

            Ok::<bool, String>(
                props
                    .get("Present")
                    .and_then(|v| bool::try_from(v.clone()).ok())
                    .unwrap_or(false),
            )
        })
        .await?;

        if present {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!("SIM not present after {}s", timeout.as_secs()));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// 等待网络注册完成
///
/// 轮询 `org.ofono.NetworkRegistration` 的 `Status` 属性，
/// 直到变为 registered / roaming，或超时。
///
/// # Returns
/// * `Ok(status)` - 已注册，返回最终状态
/// * `Err(msg)` - 超时或读取失败
async fn wait_for_network_registered(
    conn: &Connection,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;

    loop {
        let status = with_serial(async {
            let proxy = NetworkRegistrationProxy::new(conn)
                .await
                .map_err(|e| format!("NetworkRegistration unavailable: {}", e))?;

            let props = proxy
                .get_properties()
                .await
                .map_err(|e| format!("Failed to read registration: {}", e))?;

            Ok::<String, String>(
                props
                    .get("Status")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_else(|| "unknown".to_string()),
            )
        })
        .await?;

        if status == "registered" || status == "roaming" {
            return Ok(status);
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "Network not registered after {}s (status: {})",
                timeout.as_secs(),
                status
            ));
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// 自动配置 APN（根据 SIM 卡运营商）
///
/// 根据 SIM 卡的 MCC/MNC 自动查找并设置推荐的 APN 配置
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `context_path` - 要配置的 context 路径
///
/// # Returns
/// 配置结果消息
async fn auto_configure_apn(conn: &Connection, context_path: &str) -> Result<String, String> {
    // 1. 获取网络注册信息中的 MCC/MNC
    let (mcc, mnc) = read_operator_mcc_mnc(conn)
        .await
        .ok_or_else(|| "MCC/MNC not available".to_string())?;

    // 2. 查找推荐 APN
    let (apn, protocol) = get_recommended_apn(&mcc, &mnc)
        .ok_or_else(|| format!("No recommended APN for MCC={} MNC={}", mcc, mnc))?;
    
    // 3. 设置 APN 和协议
    set_apn_property(conn, context_path, "AccessPointName", apn)
        .await
        .map_err(|e| format!("Failed to set APN: {}", e))?;
    
    set_apn_property(conn, context_path, "Protocol", protocol)
        .await
        .map_err(|e| format!("Failed to set protocol: {}", e))?;
    
    Ok(format!("Auto-configured APN: {} ({})", apn, protocol))
}

/// 连续激活失败达到该次数后，进入长时间冷却
const MAX_CONSECUTIVE_FAILURES: u32 = 8;

/// 放弃重试后的冷却时长（秒），冷却结束后重新开始计数
const GIVE_UP_COOLDOWN_SECS: u64 = 300;

/// 连续失败后的退避档位（秒）
const BACKOFF_STEPS: [u64; 5] = [5, 15, 30, 60, 120];

/// 数据连接检查结果分类
///
/// 用于让 watchdog 区分「环境未就绪」和「真正的激活失败」，
/// 只有后者才计入连续失败次数并触发退避，避免网络暂时不可用时被误判为故障。
enum DataCheckOutcome {
    /// 连接正常，或本次已成功恢复
    Healthy,
    /// 环境未就绪（未注册网络、取不到 context、APN 无法确定），不计入失败
    Waiting,
    /// 激活失败，计入连续失败次数
    Failed,
}

/// Watchdog 运行状态
struct WatchdogState {
    /// 上次观察到的 SIM ICCID，用于检测换卡
    last_iccid: Option<String>,
    /// 连续激活失败次数
    consecutive_failures: u32,
    /// 下一次允许尝试激活的时间点
    next_attempt_at: Instant,
}

impl WatchdogState {
    fn new() -> Self {
        Self {
            last_iccid: None,
            consecutive_failures: 0,
            next_attempt_at: Instant::now(),
        }
    }

    /// 当前失败次数对应的退避时长（指数退避，封顶 120 秒）
    fn backoff(&self) -> Duration {
        let idx = (self.consecutive_failures as usize).min(BACKOFF_STEPS.len() - 1);
        Duration::from_secs(BACKOFF_STEPS[idx])
    }

    /// 记录一次激活失败，并计算下次尝试时间
    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);

        if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            // 连续失败太多次，说明当前 APN 或网络状态本身有问题，
            // 继续高频重试只会把 RIL 打进 "Operation already in progress" 的半死状态。
            // 这里改为长时间冷却，并清零计数，保证之后还能自动恢复而不永久卡死。
            warn!(
                failures = self.consecutive_failures,
                cooldown_secs = GIVE_UP_COOLDOWN_SECS,
                "Watchdog: too many consecutive activation failures, entering cooldown"
            );
            self.consecutive_failures = 0;
            self.next_attempt_at = Instant::now() + Duration::from_secs(GIVE_UP_COOLDOWN_SECS);
        } else {
            self.next_attempt_at = Instant::now() + self.backoff();
        }
    }

    /// 记录一次成功（或状态恢复），清零失败计数
    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.next_attempt_at = Instant::now();
    }
}

/// 物理换卡（热插拔）后的完整恢复流程。
///
/// 与 `switch_sim_slot` 的区别：卡是物理插拔的，模组已经认到新卡，
/// 不需要再发 `AT+SPCONFIGSIMSLOT`；但仍必须做「拆承载 + Modem.Online
/// 重枚举 + 等就绪/注册 + 重配 APN + 激活」，否则被硬拔卡留下的 stuck
/// PDP 承载会卡死在 RIL/运营侧，只发 `Active=true` 清不掉，只能重启。
///
/// 由 watchdog 在检测到 ICCID 变化时调用。内部各子调用自带 `with_serial`，
/// 因此这里不要再包 `with_serial`。
async fn recover_after_sim_change(
    conn: &Connection,
    state: &mut WatchdogState,
) -> (DataCheckOutcome, String) {
    // 1. 先拆掉残留的 PDP 承载，清掉 stuck bearer
    if let Err(e) = set_data_connection(conn, false).await {
        warn!(error = %e, "recover: deactivate failed (may already be down)");
    }

    // 2. 重枚举 modem，强制清除卡死态
    if let Err(e) = reenumerate_modem(conn).await {
        state.record_failure();
        return (DataCheckOutcome::Failed, format!("re-enum failed: {}", e));
    }

    // 3. 重枚举后 context 路径可能变化，重新查找并按新卡运营商重配 APN
    match find_internet_context(conn).await {
        Ok(path) => match auto_configure_apn(conn, &path).await {
            Ok(msg) => match set_data_connection(conn, true).await {
                Ok(()) => {
                    state.record_success();
                    (DataCheckOutcome::Healthy, format!("{}; re-activated after SIM change", msg))
                }
                Err(e) => {
                    state.record_failure();
                    (DataCheckOutcome::Failed, format!("{}; activate failed: {}", msg, e))
                }
            },
            Err(e) => (
                DataCheckOutcome::Waiting,
                format!("APN reconfig unavailable after SIM change: {}", e),
            ),
        },
        Err(e) => (
            DataCheckOutcome::Waiting,
            format!("no internet context after SIM change: {}", e),
        ),
    }
}

/// 让 modem 下线再上线，强制重新枚举 SIM 并清除 stuck PDP 承载。
///
/// 所有等待都在锁外进行；`Modem.Online` 的写操作为单次 D-Bus 调用，无需额外加锁。
async fn reenumerate_modem(conn: &Connection) -> Result<(), String> {
    let modem = ModemProxy::new(conn)
        .await
        .map_err(|e| format!("Modem proxy unavailable: {}", e))?;

    // 下线，强制 modem 丢弃旧 SIM 的注册与承载
    let _ = modem
        .set_property("Online", zbus::zvariant::Value::Bool(false))
        .await;

    // 短暂稳定后再上线
    tokio::time::sleep(Duration::from_secs(2)).await;

    modem
        .set_property("Online", zbus::zvariant::Value::Bool(true))
        .await
        .map_err(|e| format!("failed to bring modem online: {}", e))?;

    // 等待新卡就绪与网络注册（等待在锁外，不阻塞其它 API）
    wait_for_sim_ready(conn, SIM_READY_TIMEOUT).await?;
    wait_for_network_registered(conn, NETWORK_REGISTER_TIMEOUT).await?;

    Ok(())
}

/// 检查并恢复数据连接
///
/// 这个函数被 watchdog 调用，检查数据连接状态并在需要时恢复。
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `state` - watchdog 运行状态（换卡检测 + 退避计数）
///
/// # Returns
/// `(结果分类, 状态描述字符串)`
async fn check_and_restore_data_connection(
    conn: &Connection,
    state: &mut WatchdogState,
    usage: &DataUsageTracker,
) -> (DataCheckOutcome, String) {
    // 0. 流量已达限额且开启自动关闭 → 不自动恢复，避免刚关又被拉起
    if usage.is_blocked() {
        return (
            DataCheckOutcome::Waiting,
            "Data connection blocked by data limit; not auto-activating".to_string(),
        );
    }

    // 1. 检查网络注册状态
    let net_status = match NetworkRegistrationProxy::new(conn).await {
        Ok(net_proxy) => {
            match net_proxy.get_properties().await {
                Ok(props) => props
                    .get("Status")
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_else(|| "unknown".to_string()),
                Err(_) => "unknown".to_string(),
            }
        }
        Err(_) => return (DataCheckOutcome::Waiting, "Network proxy unavailable".to_string()),
    };
    
    // 网络未注册时不尝试恢复，这只是等待，不是故障，不计入失败
    if net_status != "registered" && net_status != "roaming" {
        return (
            DataCheckOutcome::Waiting,
            format!("Waiting for network (status: {})", net_status),
        );
    }
    
    // 2. 检测换卡
    //    ICCID 唯一标识一张 SIM 卡，它变化就说明卡被换了。
    //    此时旧卡遗留的 APN 对新卡几乎必然无效，必须按新卡运营商重配 ——
    //    这正是「能读卡但激活不了蜂窝数据」的根因所在。
    let iccid = read_sim_iccid(conn).await;
    let sim_changed = match (&state.last_iccid, &iccid) {
        (Some(prev), Some(cur)) => prev != cur,
        _ => false,
    };
    if iccid.is_some() {
        state.last_iccid = iccid;
    }
    if sim_changed {
        info!("Watchdog: SIM changed, performing full re-enumeration + APN reconfig");
        // 物理换卡（热插拔）绕过了 switch_sim_slot 的 Modem.Online 重枚举，
        // 被硬拔卡留下的 stuck PDP 承载会卡死在 RIL/运营侧，只发 Active=true 清不掉，
        // 只能靠整机重启。这里复用重枚举流程：拆承载 → Online 重枚举 → 等就绪/注册 →
        // 重配 APN → 激活，让热插拔也能自愈，不再依赖重启。
        return recover_after_sim_change(conn, state).await;
    }
    
    // 3. 查找 internet context
    let context_path = match find_internet_context(conn).await {
        Ok(path) => path,
        Err(e) => return (DataCheckOutcome::Waiting, format!("No internet context: {}", e)),
    };
    
    // 4. 获取 context 属性
    let proxy = match ConnectionContextProxy::builder(conn)
        .path(context_path.as_str())
        .and_then(|b| Ok(b))
    {
        Ok(builder) => match builder.build().await {
            Ok(p) => p,
            Err(e) => return (DataCheckOutcome::Waiting, format!("Context proxy error: {}", e)),
        },
        Err(e) => return (DataCheckOutcome::Waiting, format!("Context path error: {}", e)),
    };
    
    let props = match proxy.get_properties().await {
        Ok(p) => p,
        Err(e) => return (DataCheckOutcome::Waiting, format!("Get properties error: {}", e)),
    };
    
    let apn = props
        .get("AccessPointName")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();
    
    let active = props
        .get("Active")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    
    // 5. 判定是否需要重配 APN
    //    - APN 为空：新卡还没配过
    //    - 检测到换卡：旧卡 APN 对新卡无效，必须覆盖
    //
    //    注意：不会因为「APN 与推荐值不一致」就覆盖，
    //    否则会冲掉用户手动设置的自定义 APN。
    let need_apn_reconfig = apn.is_empty() || sim_changed;
    
    if need_apn_reconfig {
        let reason = if apn.is_empty() {
            "APN not configured"
        } else {
            "SIM changed"
        };
        
        match auto_configure_apn(conn, &context_path).await {
            Ok(msg) => {
                // APN 配置成功后，继续尝试激活
                match set_data_connection(conn, true).await {
                    Ok(_) => {
                        state.record_success();
                        return (
                            DataCheckOutcome::Healthy,
                            format!("{} ({}), connection activated", msg, reason),
                        );
                    }
                    Err(e) => {
                        state.record_failure();
                        return (
                            DataCheckOutcome::Failed,
                            format!("{} ({}), but activation failed: {}", msg, reason, e),
                        );
                    }
                }
            }
            Err(e) => {
                // 无法根据 MCC/MNC 推断 APN（境外卡或注册信息还没就绪），
                // 此时不要拿可能错误的旧 APN 反复重试，等下次再试
                return (
                    DataCheckOutcome::Waiting,
                    format!("APN auto-config unavailable ({}): {}", reason, e),
                );
            }
        }
    }
    
    // 6. 如果连接未激活，尝试激活
    if !active {
        match set_data_connection(conn, true).await {
            Ok(_) => {
                state.record_success();
                return (
                    DataCheckOutcome::Healthy,
                    format!("Connection restored (APN: {})", apn),
                );
            }
            Err(e) => {
                state.record_failure();
                return (DataCheckOutcome::Failed, format!("Activation failed: {}", e));
            }
        }
    }
    
    // 7. 连接正常
    state.record_success();
    (DataCheckOutcome::Healthy, format!("Connected (APN: {})", apn))
}

/// 采样流量并强制生效流量限额
///
/// - 限额为 0（未设置）时不限制；
/// - 限额 > 0 且 `auto_disable` 开启，且累计流量 >= 限额时：强制关闭数据连接并置阻断标志；
/// - 否则清除阻断标志（允许手动开启 / 自动恢复）。
async fn enforce_data_limit(
    conn: &Connection,
    config: &ConfigManager,
    usage: &DataUsageTracker,
) {
    let (rx, tx) = usage.sample();
    let cfg = config.get_data_connection_config();

    if cfg.limit_gb <= 0.0 {
        usage.set_blocked(false);
        return;
    }

    let limit_bytes = (cfg.limit_gb * crate::usage::BYTES_PER_GB as f64) as u64;
    let total = rx.saturating_add(tx);

    if cfg.auto_disable && total >= limit_bytes {
        // 强制关闭并标记阻断（阻断后 check_and_restore_data_connection 不会再自动拉起）
        if let Ok(true) = get_data_connection_status(conn).await {
            let _ = set_data_connection(conn, false).await;
        }
        usage.set_blocked(true);
        info!(
            used_bytes = total,
            used_gb = total as f64 / crate::usage::BYTES_PER_GB as f64,
            limit_gb = cfg.limit_gb,
            "流量已达限额，自动关闭数据连接"
        );
    } else {
        usage.set_blocked(false);
    }
}

/// 数据连接 Watchdog - 后台轮询监控并自动恢复
///
/// 持续监控数据连接状态，在断开时自动尝试恢复。
/// 支持自动识别运营商并配置 APN。
///
/// 失败重试采用指数退避（5s → 15s → 30s → 60s → 120s），连续失败 8 次后进入
/// 5 分钟冷却，避免高频重试把 RIL 打进 "Operation already in progress" 的半死状态。
/// 检测到换卡（ICCID 变化）会立即清零退避并按新卡运营商重配 APN。
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `interval_secs` - 基础轮询间隔（秒），仅用于 iptables 检查和退避计时精度
/// * `config` - 配置管理器（读取流量限额设置）
/// * `usage` - 数据流量追踪器（采样累计 + 限额阻断状态）
pub async fn data_connection_watchdog(
    conn: std::sync::Arc<Connection>,
    interval_secs: u64,
    config: std::sync::Arc<ConfigManager>,
    usage: std::sync::Arc<DataUsageTracker>,
) {
    use crate::iptables::{flush_iptables, get_iptables_rule_count};

    let mut last_data_log = String::new();
    let mut last_iptables_action = false; // 上次是否清空了 iptables
    let mut state = WatchdogState::new();

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;

        // 1. 检查并清空 iptables 规则
        match get_iptables_rule_count().await {
            Ok(count) => {
                if count.has_rules() {
                    // 有规则，执行清空
                    if let Err(e) = flush_iptables().await {
                        warn!(error = %e, "Watchdog: iptables flush failed");
                    } else {
                        if !last_iptables_action {
                            // 只在首次清空时打印日志
                            info!(
                                total = count.total(),
                                ipv4 = count.ipv4_rules,
                                ipv6 = count.ipv6_rules,
                                "Watchdog: iptables flushed"
                            );
                        }
                        last_iptables_action = true;
                    }
                } else {
                    // 无规则，重置标志
                    last_iptables_action = false;
                }
            }
            Err(e) => {
                warn!(error = %e, "Watchdog: iptables check failed");
            }
        }

        // 1.5 采样流量并强制生效流量限额（每轮都做，即便下面因退避跳过恢复，
        //     这样累计值不会在退避窗口内漏计）
        enforce_data_limit(&conn, &config, &usage).await;

        // 2. 检查并恢复数据连接
        //    按退避节奏调度：连续失败后逐步拉长尝试间隔，
        //    未到下次尝试时间就跳过本轮，只保留 iptables 检查的原有节奏
        if Instant::now() < state.next_attempt_at {
            continue;
        }

        let (outcome, result) = check_and_restore_data_connection(&conn, &mut state, &usage).await;
        
        match outcome {
            DataCheckOutcome::Failed => {
                // 失败始终打印，便于现场定位；同时清空 last_data_log，
                // 保证下次恢复到正常状态时一定会被记录
                warn!(
                    status = %result,
                    consecutive_failures = state.consecutive_failures,
                    "Watchdog: data connection failed"
                );
                last_data_log.clear();
            }
            _ => {
                // 只在状态变化时打印日志，避免刷屏
                if result != last_data_log {
                    info!(status = %result, "Watchdog: data connection");
                    last_data_log = result;
                }
            }
        }
    }
}

/// 获取 SIM 卡信息（整合所有 SIM 相关信息）
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// SIM 卡信息结构（整合 SimManager + MessageManager）
pub async fn get_sim_info_data(conn: &Connection) -> zbus::Result<SimInfoResponse> {
    let sim_proxy = SimManagerProxy::new(conn).await?;
    let msg_proxy = MessageManagerProxy::new(conn).await?;
    
    let sim_props = sim_proxy.get_properties().await?;
    let msg_props = msg_proxy.get_properties().await?;

    // 基本状态
    let present = sim_props
        .get("Present")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);

    // ICCID
    let iccid = sim_props
        .get("CardIdentifier")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    // IMSI
    let imsi = sim_props
        .get("SubscriberIdentity")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    // 手机号码列表
    let phone_numbers: Vec<String> = sim_props
        .get("SubscriberNumbers")
        .and_then(|v| <Vec<String>>::try_from(v.clone()).ok())
        .unwrap_or_default();

    // 短信中心
    let sms_center = msg_props
        .get("ServiceCenterAddress")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    // MCC/MNC
    let mcc = sim_props
        .get("MobileCountryCode")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    let mnc = sim_props
        .get("MobileNetworkCode")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    // PIN 状态
    let pin_required = sim_props
        .get("PinRequired")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "none".to_string());

    // 首选语言
    let preferred_languages: Vec<String> = sim_props
        .get("PreferredLanguages")
        .and_then(|v| <Vec<String>>::try_from(v.clone()).ok())
        .unwrap_or_default();

    Ok(SimInfoResponse {
        present,
        iccid,
        imsi,
        phone_numbers,
        sms_center,
        mcc,
        mnc,
        pin_required,
        preferred_languages,
    })
}

/// 获取网络信息
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// 网络信息结构
pub async fn get_network_info_data(conn: &Connection) -> zbus::Result<NetworkInfoResponse> {
    let net_proxy = NetworkRegistrationProxy::new(conn).await?;
    let radio_proxy = RadioSettingsProxy::new(conn).await?;
    
    let net_props = net_proxy.get_properties().await?;
    let radio_props = radio_proxy.get_properties().await?;

    let operator_name = net_props
        .get("Name")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    let registration_status = net_props
        .get("Status")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "unknown".to_string());

    let technology_preference = radio_props
        .get("TechnologyPreference")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    let signal_strength = net_props
        .get("Strength")
        .and_then(|v| u8::try_from(v.clone()).ok())
        .unwrap_or(0);

    let mcc = net_props
        .get("MobileCountryCode")
        .and_then(|v| String::try_from(v.clone()).ok());

    let mnc = net_props
        .get("MobileNetworkCode")
        .and_then(|v| String::try_from(v.clone()).ok());

    Ok(NetworkInfoResponse {
        operator_name,
        registration_status,
        technology_preference,
        signal_strength,
        mcc,
        mnc,
    })
}

/// 从系统真实来源读取设备标识（device-tree / os-release），
/// 用于替换 modem D-Bus 返回的 "Fake Modem" 等占位符。
/// 仅在 D-Bus 返回空或包含 "fake" 时调用。
fn read_real_device_identity() -> Option<(String, String)> {
    // 1) device-tree model，例如 "Soyea UDX710"
    if let Ok(s) = std::fs::read_to_string("/proc/device-tree/model") {
        let s = s.trim_end_matches('\0').trim().to_string();
        if !s.is_empty() && !s.to_lowercase().contains("fake") {
            let manufacturer = s
                .split_whitespace()
                .next()
                .unwrap_or("Unknown")
                .to_string();
            return Some((manufacturer, s));
        }
    }
    // 2) /etc/os-release PRETTY_NAME，例如 "OpenWrt 22.03"
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                let val = val.trim_matches('"').to_string();
                if !val.is_empty() && !val.to_lowercase().contains("fake") {
                    return Some(("Unknown".to_string(), val));
                }
            }
        }
    }
    None
}

/// 获取设备信息（来自 D-Bus Modem 接口）
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// 设备信息结构
pub async fn get_device_info_data(conn: &Connection) -> zbus::Result<DeviceInfoResponse> {
    let proxy = ModemProxy::new(conn).await?;
    let props = proxy.get_properties().await?;

    let imei = props
        .get("Serial")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    let mut manufacturer = props
        .get("Manufacturer")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    let mut model = props
        .get("Model")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();

    // 若 modem 返回占位符（Fake）或为空，尝试系统真实来源
    let looks_fake = |s: &str| s.is_empty() || s.to_lowercase().contains("fake");
    if looks_fake(&manufacturer) || looks_fake(&model) {
        if let Some((real_mfr, real_model)) = read_real_device_identity() {
            if looks_fake(&manufacturer) {
                manufacturer = real_mfr;
            }
            if looks_fake(&model) {
                model = real_model;
            }
        }
    }

    let revision = props
        .get("Revision")
        .and_then(|v| String::try_from(v.clone()).ok());

    // 固件版本：优先用 modem 的 revision，缺失时回退到构建版本号（真实可控）
    let firmware_version = revision
        .clone()
        .filter(|r| !r.is_empty())
        .or_else(|| Some(env!("CARGO_PKG_VERSION").to_string()));

    let online = props
        .get("Online")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);

    let powered = props
        .get("Powered")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);

    Ok(DeviceInfoResponse {
        imei,
        manufacturer,
        model,
        revision,
        firmware_version,
        online,
        powered,
    })
}

/// 获取QoS信息
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// QoS信息结构
pub async fn get_qos_info_data(conn: &Connection) -> zbus::Result<QosInfoResponse> {
    let response = send_at_command(conn, "AT+CGEQOSRDP").await?;
    
    // 解析 +CGEQOSRDP: <cid>,<QCI>,[<DL_GBR>,<UL_GBR>],[<DL_MBR>,<UL_MBR>],[<DL_AMBR>,<UL_AMBR>]
    let parsed = parse_qos_response(&response);
    
    Ok(parsed)
}

/// 解析QoS响应
///
/// 格式: +CGEQOSRDP: <cid>,<QCI>,[<DL_GBR>,<UL_GBR>],[<DL_MBR>,<UL_MBR>],[<DL_AMBR>,<UL_AMBR>]
/// 示例: +CGEQOSRDP: 11,5,0,0,0,0,30000,30000
fn parse_qos_response(response: &str) -> QosInfoResponse {
    // 查找 +CGEQOSRDP: 开头的行
    for line in response.lines() {
        let line = line.trim();
        if line.starts_with("+CGEQOSRDP:") {
            // 提取冒号后面的部分
            if let Some(data) = line.strip_prefix("+CGEQOSRDP:") {
                let parts: Vec<&str> = data.trim().split(',').collect();
                
                if parts.len() >= 8 {
                    // 解析各个字段
                    let qci = parts.get(1).and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(0);
                    let dl_gbr = parts.get(2).and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
                    let ul_gbr = parts.get(3).and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
                    let dl_mbr = parts.get(4).and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
                    let ul_mbr = parts.get(5).and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
                    let dl_ambr = parts.get(6).and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
                    let ul_ambr = parts.get(7).and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(0);
                    
                    // 优先使用 GBR，如果为0则使用 MBR，如果还是0则使用 AMBR
                    let dl_speed = if dl_gbr > 0 { dl_gbr } else if dl_mbr > 0 { dl_mbr } else { dl_ambr };
                    let ul_speed = if ul_gbr > 0 { ul_gbr } else if ul_mbr > 0 { ul_mbr } else { ul_ambr };
                    
                    return QosInfoResponse {
                        qci,
                        dl_speed,
                        ul_speed,
                        raw_response: None, // 不返回原始响应，保持简洁
                    };
                }
            }
        }
    }
    
    // 如果解析失败，返回默认值
    QosInfoResponse {
        qci: 0,
        dl_speed: 0,
        ul_speed: 0,
        raw_response: Some(response.to_string()),
    }
}

/// 从多个可能的键中解析 u32 值
///
/// 不同的 udx710 设备可能使用不同的键名和值类型
fn parse_u32_from_keys(cell_info: &HashMap<String, OwnedValue>, keys: &[&str]) -> u32 {
    for key in keys {
        if let Some(value) = cell_info.get(*key) {
            // 尝试直接转换为 u32
            if let Ok(num) = u32::try_from(value) {
                return num;
            }
            // 尝试转换为字符串后再解析
            if let Ok(s) = String::try_from(value.clone()) {
                // 尝试十进制解析
                if let Ok(num) = s.parse::<u32>() {
                    return num;
                }
                // 尝试十六进制解析
                if let Ok(num) = u32::from_str_radix(&s, 16) {
                    return num;
                }
            }
        }
    }
    0
}

/// 设置飞行模式
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `enabled` - true 开启飞行模式（关闭射频），false 关闭飞行模式（开启射频）
///
/// # Returns
/// 操作结果
///
/// # 说明
/// 飞行模式通过设置 Modem 的 Online 属性实现：
/// - Online = false: 关闭射频，进入飞行模式（但 Modem 保持上电）
/// - Online = true: 开启射频，退出飞行模式
pub async fn set_airplane_mode(conn: &Connection, enabled: bool) -> zbus::Result<()> {
    with_serial(async {
        let proxy = ModemProxy::new(conn).await?;
        
        // 飞行模式：设置 Online 为相反值
        // enabled=true 表示开启飞行模式，即 Online=false
        proxy
            .set_property("Online", zbus::zvariant::Value::Bool(!enabled))
            .await?;
        
        Ok(())
    }).await
}

/// 获取飞行模式状态
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// 飞行模式响应结构，包含飞行模式状态、Powered 和 Online 属性
///
/// # 说明
/// 飞行模式状态判断：
/// - enabled = !Online (Online=false 表示飞行模式已启用)
pub async fn get_airplane_mode(conn: &Connection) -> zbus::Result<AirplaneModeResponse> {
    let proxy = ModemProxy::new(conn).await?;
    let props = proxy.get_properties().await?;
    
    let powered = props
        .get("Powered")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    
    let online = props
        .get("Online")
        .and_then(|v| bool::try_from(v.clone()).ok())
        .unwrap_or(false);
    
    // 飞行模式状态：Online=false 表示飞行模式已启用
    let enabled = !online;
    
    Ok(AirplaneModeResponse {
        enabled,
        powered,
        online,
    })
}

/// 获取射频模式
///
/// # Arguments
/// * `conn` - D-Bus 连接
///
/// # Returns
/// 射频模式响应结构
///
/// # 说明
/// 通过 RadioSettings.GetProperties 获取 TechnologyPreference 属性
pub async fn get_radio_mode(conn: &Connection) -> zbus::Result<RadioModeResponse> {
    with_serial(async {
        let proxy = RadioSettingsProxy::new(conn).await?;
        let props = proxy.get_properties().await?;
        
        let technology_preference = props
            .get("TechnologyPreference")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        // 尝试映射为标准模式
        let mode = RadioMode::from_ofono_value(&technology_preference)
            .map(|m| match m {
                RadioMode::Auto => "auto",
                RadioMode::LteOnly => "lte",
                RadioMode::NrOnly => "nr",
            })
            .unwrap_or("unknown")
            .to_string();
        
        Ok(RadioModeResponse {
            mode,
            technology_preference,
        })
    }).await
}

/// 设置射频模式
///
/// # Arguments
/// * `conn` - D-Bus 连接
/// * `mode` - 目标射频模式
///
/// # Returns
/// 操作结果
///
/// # 说明
/// 通过 RadioSettings.SetProperty 设置 TechnologyPreference 属性
pub async fn set_radio_mode(conn: &Connection, mode: RadioMode) -> zbus::Result<()> {
    with_serial(async {
        let proxy = RadioSettingsProxy::new(conn).await?;
        let ofono_value = mode.to_ofono_value();
        
        proxy
            .set_property(
                "TechnologyPreference",
                zbus::zvariant::Value::Str(ofono_value.into()),
            )
            .await?;
        
        Ok(())
    }).await
}

// ============ 电话相关 D-Bus 接口 ============

use crate::models::CallInfo;

/// ofono VoiceCallManager 代理接口
#[proxy(
    interface = "org.ofono.VoiceCallManager",
    default_service = "org.ofono",
    default_path = "/ril_0",
    assume_defaults = true
)]
pub trait VoiceCallManager {
    /// 获取所有通话
    fn get_calls(&self) -> zbus::Result<Vec<(zbus::zvariant::OwnedObjectPath, HashMap<String, OwnedValue>)>>;
    
    /// 拨打电话
    fn dial(&self, number: &str, hide_callerid: &str) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
    
    /// 挂断所有通话
    fn hangup_all(&self) -> zbus::Result<()>;
}

/// ofono VoiceCall 代理接口（单个通话）
#[proxy(
    interface = "org.ofono.VoiceCall",
    default_service = "org.ofono",
    assume_defaults = true
)]
pub trait VoiceCall {
    /// 挂断此通话
    fn hangup(&self) -> zbus::Result<()>;
    
    /// 接听来电
    fn answer(&self) -> zbus::Result<()>;
    
    /// 获取通话属性
    fn get_properties(&self) -> zbus::Result<HashMap<String, OwnedValue>>;
}

/// 获取当前活动的通话列表
pub async fn get_active_calls(conn: &Connection) -> zbus::Result<Vec<CallInfo>> {
    with_serial(async {
        let proxy = VoiceCallManagerProxy::new(conn).await?;
        let calls = proxy.get_calls().await?;
        
        let mut result = Vec::new();
        for (path, props) in calls {
            let phone_number = props
                .get("LineIdentification")
                .and_then(|v| String::try_from(v.clone()).ok())
                .unwrap_or_else(|| "Unknown".to_string());
            
            let state = props
                .get("State")
                .and_then(|v| String::try_from(v.clone()).ok())
                .unwrap_or_else(|| "unknown".to_string());
            
            let start_time = props
                .get("StartTime")
                .and_then(|v| String::try_from(v.clone()).ok());
            
            // 判断方向：incoming 或 outgoing
            let direction = if state == "incoming" {
                "incoming".to_string()
            } else {
                "outgoing".to_string()
            };
            
            result.push(CallInfo {
                path: path.to_string(),
                phone_number,
                state,
                direction,
                start_time,
            });
        }
        
        Ok(result)
    }).await
}

/// 拨打电话
pub async fn dial_call(conn: &Connection, phone_number: &str) -> zbus::Result<CallInfo> {
    with_serial(async {
        let proxy = VoiceCallManagerProxy::new(conn).await?;
        let path = proxy.dial(phone_number, "default").await?;
        
        Ok(CallInfo {
            path: path.to_string(),
            phone_number: phone_number.to_string(),
            state: "dialing".to_string(),
            direction: "outgoing".to_string(),
            start_time: Some(chrono::Utc::now().to_rfc3339()),
        })
    }).await
}

/// 挂断指定通话
pub async fn hangup_call(conn: &Connection, call_path: &str) -> zbus::Result<()> {
    with_serial(async {
        let proxy = VoiceCallProxy::builder(conn)
            .path(call_path)?
            .build()
            .await?;
        
        proxy.hangup().await
    }).await
}

/// 挂断所有通话
pub async fn hangup_all_calls(conn: &Connection) -> zbus::Result<usize> {
    with_serial(async {
        let proxy = VoiceCallManagerProxy::new(conn).await?;
        let calls = proxy.get_calls().await?;
        let count = calls.len();
        
        if count > 0 {
            proxy.hangup_all().await?;
        }
        
        Ok(count)
    }).await
}

/// 接听来电
pub async fn answer_call(conn: &Connection, call_path: &str) -> zbus::Result<()> {
    with_serial(async {
        let proxy = VoiceCallProxy::builder(conn)
            .path(call_path)?
            .build()
            .await?;
        
        proxy.answer().await
    }).await
}

// ============ 短信相关 D-Bus 接口 ============

/// 发送短信
pub async fn send_sms(conn: &Connection, phone_number: &str, content: &str) -> zbus::Result<String> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.MessageManager").await?;
        let message_path: zbus::zvariant::OwnedObjectPath = proxy.call("SendMessage", &(phone_number, content)).await?;
        Ok(message_path.to_string())
    }).await
}

// ============ 新增功能接口 ============

use crate::models::{
    ImeisvResponse, SignalStrengthResponse, CallForwardingResponse, CallSettingsResponse,
    OperatorInfo, OperatorListResponse, NitzTimeResponse, ImsStatusResponse,
    CallVolumeResponse, VoicemailStatusResponse,
};

/// 获取 IMEISV（软件版本号）
pub async fn get_imeisv(conn: &Connection) -> zbus::Result<ImeisvResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.Modem").await?;
        let result: HashMap<String, OwnedValue> = proxy.call("GetImeisv", &()).await?;
        
        let svn = result
            .get("SoftwareVersionNumber")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "Unknown".to_string());
        
        Ok(ImeisvResponse {
            software_version_number: svn,
        })
    }).await
}

/// 获取信号强度详细信息
pub async fn get_signal_strength(conn: &Connection) -> zbus::Result<SignalStrengthResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.NetworkRegistration").await?;
        let result: HashMap<String, OwnedValue> = proxy.call("GetSignalStrength", &()).await?;
        
        let strength = result
            .get("Strength")
            .and_then(|v| i32::try_from(v.clone()).ok())
            .unwrap_or(0);
        
        Ok(SignalStrengthResponse { strength })
    }).await
}

/// 获取 NITZ 网络时间
pub async fn get_nitz_time(conn: &Connection) -> zbus::Result<NitzTimeResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.Modem").await?;
        
        match proxy.call("GetNITZ", &()).await {
            Ok(time_string) => Ok(NitzTimeResponse {
                time_string,
                available: true,
            }),
            Err(_) => Ok(NitzTimeResponse {
                time_string: String::new(),
                available: false,
            }),
        }
    }).await
}

/// 获取 IMS 状态
pub async fn get_ims_status(conn: &Connection) -> zbus::Result<ImsStatusResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.IpMultimediaSystem").await?;
        let props: HashMap<String, OwnedValue> = proxy.call("GetProperties", &()).await?;
        
        let registered = props
            .get("Registered")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        
        let voice_capable = props
            .get("VoiceCapable")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        
        let sms_capable = props
            .get("SmsCapable")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        
        Ok(ImsStatusResponse {
            registered,
            voice_capable,
            sms_capable,
        })
    }).await
}

/// 获取通话音量
pub async fn get_call_volume(conn: &Connection) -> zbus::Result<CallVolumeResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.CallVolume").await?;
        let props: HashMap<String, OwnedValue> = proxy.call("GetProperties", &()).await?;
        
        let speaker_volume = props
            .get("SpeakerVolume")
            .and_then(|v| u8::try_from(v.clone()).ok())
            .unwrap_or(0);
        
        let microphone_volume = props
            .get("MicrophoneVolume")
            .and_then(|v| u8::try_from(v.clone()).ok())
            .unwrap_or(0);
        
        let muted = props
            .get("Muted")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        
        Ok(CallVolumeResponse {
            speaker_volume,
            microphone_volume,
            muted,
        })
    }).await
}

/// 设置通话音量
pub async fn set_call_volume(
    conn: &Connection,
    speaker: Option<u8>,
    microphone: Option<u8>,
    muted: Option<bool>,
) -> zbus::Result<()> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.CallVolume").await?;
        
        if let Some(vol) = speaker {
            let val = zbus::zvariant::Value::new(vol);
            proxy.call::<_, _, ()>("SetProperty", &("SpeakerVolume", val)).await?;
        }
        
        if let Some(vol) = microphone {
            let val = zbus::zvariant::Value::new(vol);
            proxy.call::<_, _, ()>("SetProperty", &("MicrophoneVolume", val)).await?;
        }
        
        if let Some(m) = muted {
            let val = zbus::zvariant::Value::new(m);
            proxy.call::<_, _, ()>("SetProperty", &("Muted", val)).await?;
        }
        
        Ok(())
    }).await
}

/// 获取语音留言状态
pub async fn get_voicemail_status(conn: &Connection) -> zbus::Result<VoicemailStatusResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.MessageWaiting").await?;
        let props: HashMap<String, OwnedValue> = proxy.call("GetProperties", &()).await?;
        
        let waiting = props
            .get("VoicemailWaiting")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        
        let message_count = props
            .get("VoicemailMessageCount")
            .and_then(|v| u8::try_from(v.clone()).ok())
            .unwrap_or(0);
        
        let mailbox_number = props
            .get("VoicemailMailboxNumber")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| String::new());
        
        Ok(VoicemailStatusResponse {
            waiting,
            message_count,
            mailbox_number,
        })
    }).await
}

/// 获取运营商列表（快速，仅返回当前）
pub async fn get_operators(conn: &Connection) -> zbus::Result<OperatorListResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.NetworkRegistration").await?;
        let result: Vec<(zbus::zvariant::OwnedObjectPath, HashMap<String, OwnedValue>)> = 
            proxy.call("GetOperators", &()).await?;
        
        let mut operators = Vec::new();
        for (path, props) in result {
            operators.push(parse_operator_info(path.to_string(), props));
        }
        
        Ok(OperatorListResponse { operators })
    }).await
}

/// 扫描运营商（慢，返回所有可用）
pub async fn scan_operators(conn: &Connection) -> zbus::Result<OperatorListResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.NetworkRegistration").await?;
        let result: Vec<(zbus::zvariant::OwnedObjectPath, HashMap<String, OwnedValue>)> = 
            proxy.call("Scan", &()).await?;
        
        let mut operators = Vec::new();
        for (path, props) in result {
            operators.push(parse_operator_info(path.to_string(), props));
        }
        
        Ok(OperatorListResponse { operators })
    }).await
}

/// 解析运营商信息
fn parse_operator_info(path: String, props: HashMap<String, OwnedValue>) -> OperatorInfo {
    let name = props
        .get("Name")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "Unknown".to_string());
    
    let status = props
        .get("Status")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "unknown".to_string());
    
    let mcc = props
        .get("MobileCountryCode")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "".to_string());
    
    let mnc = props
        .get("MobileNetworkCode")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "".to_string());
    
    let technologies: Vec<String> = props
        .get("Technologies")
        .and_then(|v| {
            // 尝试将 Value 转换为数组
            if let Ok(arr) = <Vec<String>>::try_from(v.clone()) {
                Some(arr)
            } else {
                None
            }
        })
        .unwrap_or_else(Vec::new);
    
    OperatorInfo {
        path,
        name,
        status,
        mcc,
        mnc,
        technologies,
    }
}

/// 手动注册到指定运营商
pub async fn register_operator_manual(conn: &Connection, mccmnc: &str) -> zbus::Result<()> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.NetworkRegistration").await?;
        proxy.call("RegisterManually", &(mccmnc, "")).await
    }).await
}

/// 自动注册运营商
pub async fn register_operator_auto(conn: &Connection) -> zbus::Result<()> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.NetworkRegistration").await?;
        proxy.call("Register", &()).await
    }).await
}

/// 获取呼叫转移设置
pub async fn get_call_forwarding(conn: &Connection) -> zbus::Result<CallForwardingResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.CallForwarding").await?;
        let props: HashMap<String, OwnedValue> = proxy.call("GetProperties", &()).await?;
        
        let voice_unconditional = props
            .get("VoiceUnconditional")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| String::new());
        
        let voice_busy = props
            .get("VoiceBusy")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| String::new());
        
        let voice_no_reply = props
            .get("VoiceNoReply")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| String::new());
        
        let voice_no_reply_timeout = props
            .get("VoiceNoReplyTimeout")
            .and_then(|v| u16::try_from(v.clone()).ok())
            .unwrap_or(20);
        
        let voice_not_reachable = props
            .get("VoiceNotReachable")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| String::new());
        
        let forwarding_flag_on_sim = props
            .get("ForwardingFlagOnSim")
            .and_then(|v| bool::try_from(v.clone()).ok())
            .unwrap_or(false);
        
        Ok(CallForwardingResponse {
            voice_unconditional,
            voice_busy,
            voice_no_reply,
            voice_no_reply_timeout,
            voice_not_reachable,
            forwarding_flag_on_sim,
        })
    }).await
}

/// 设置呼叫转移
pub async fn set_call_forwarding(
    conn: &Connection,
    forward_type: &str,
    number: &str,
    timeout: Option<u16>,
) -> zbus::Result<()> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.CallForwarding").await?;
        
        let property_name = match forward_type {
            "unconditional" => "VoiceUnconditional",
            "busy" => "VoiceBusy",
            "noreply" => "VoiceNoReply",
            "notreachable" => "VoiceNotReachable",
            _ => return Err(zbus::Error::Failure("Invalid forward type".to_string())),
        };
        
        let number_value = zbus::zvariant::Value::new(number);
        proxy.call::<_, _, ()>("SetProperty", &(property_name, number_value)).await?;
        
        // 如果是 noreply 类型且提供了超时时间
        if forward_type == "noreply" && timeout.is_some() {
            let timeout_value = zbus::zvariant::Value::new(timeout.unwrap());
            proxy.call::<_, _, ()>("SetProperty", &("VoiceNoReplyTimeout", timeout_value)).await?;
        }
        
        Ok(())
    }).await
}

/// 获取通话设置
pub async fn get_call_settings(conn: &Connection) -> zbus::Result<CallSettingsResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.CallSettings").await?;
        let props: HashMap<String, OwnedValue> = proxy.call("GetProperties", &()).await?;
        
        let calling_line_presentation = props
            .get("CallingLinePresentation")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        let calling_name_presentation = props
            .get("CallingNamePresentation")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        let connected_line_presentation = props
            .get("ConnectedLinePresentation")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        let connected_line_restriction = props
            .get("ConnectedLineRestriction")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        let called_line_presentation = props
            .get("CalledLinePresentation")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        let calling_line_restriction = props
            .get("CallingLineRestriction")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        let hide_caller_id = props
            .get("HideCallerId")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "default".to_string());
        
        let voice_call_waiting = props
            .get("VoiceCallWaiting")
            .and_then(|v| String::try_from(v.clone()).ok())
            .unwrap_or_else(|| "unknown".to_string());
        
        Ok(CallSettingsResponse {
            calling_line_presentation,
            calling_name_presentation,
            connected_line_presentation,
            connected_line_restriction,
            called_line_presentation,
            calling_line_restriction,
            hide_caller_id,
            voice_call_waiting,
        })
    }).await
}

/// 设置通话设置
pub async fn set_call_setting(conn: &Connection, property: &str, value: &str) -> zbus::Result<()> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.CallSettings").await?;
        let value_variant = zbus::zvariant::Value::new(value);
        proxy.call("SetProperty", &(property, value_variant)).await
    }).await
}

// ============ SIM 卡槽功能 ============

use crate::models::SimSlotResponse;

/// AT+SPCONFIGSIMSLOT 中代表卡槽 1 的参数值
const SIM_SLOT_1_VALUE: &str = "66051";
/// AT+SPCONFIGSIMSLOT 中代表卡槽 2 的参数值
const SIM_SLOT_2_VALUE: &str = "66306";

/// 切卡后等待 SIM 卡就绪的超时
const SIM_READY_TIMEOUT: Duration = Duration::from_secs(20);
/// 切卡后等待网络注册的超时
const NETWORK_REGISTER_TIMEOUT: Duration = Duration::from_secs(40);

/// 获取 SIM 卡槽信息
pub async fn get_sim_slot(conn: &Connection) -> zbus::Result<SimSlotResponse> {
    with_serial(async {
        let proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.Modem").await?;
        let response: String = proxy.call("SendAtcmd", &("AT+SPCONFIGSIMSLOT?")).await?;
        
        // 解析响应：+SPCONFIGSIMSLOT: 66051
        let raw_value = response
            .lines()
            .find(|line| line.contains("+SPCONFIGSIMSLOT:"))
            .and_then(|line| line.split_once(':'))
            .map(|(_, value)| value.trim().to_string())
            .unwrap_or_default();
        
        // 按数值解析卡槽，避免用字符串包含匹配导致误判
        // （例如 66306 里包含子串 "306"，而 66051 与 66306 本身也可能互相干扰）
        let active_slot = match raw_value.parse::<u32>() {
            Ok(66051) => 1,
            Ok(66306) => 2,
            _ => 0, // 0 表示无法识别，调用方应据此提示而不是盲切
        };
        
        Ok(SimSlotResponse {
            active_slot,
            raw_value,
        })
    }).await
}

/// 切换 SIM 卡槽
///
/// 这是一个完整的热切换流程，而不只是一条 AT 指令。原来只发指令、不做收尾，
/// 会导致 ofono 的 gprs context 一直残留旧卡的 APN 和 PDP 会话，
/// 表现为「SIM 能读到，但蜂窝数据永远激活不了，重启也没用」。
///
/// ## 完整流程
/// 1. 拆掉当前 PDP 连接，避免旧卡会话残留
/// 2. 让 modem 下线（Online=false），强制其重新读卡
/// 3. 发送切卡指令，并校验 AT 响应（原来完全不校验，ERROR 也当成功）
/// 4. modem 上线（Online=true）
/// 5. 轮询等待 SIM 就绪（Present=true）
/// 6. 轮询等待网络注册（registered / roaming）
/// 7. 按新卡的 MCC/MNC 重新配置 APN，覆盖旧卡残留值
/// 8. 重新激活数据连接
///
/// ## 锁的注意事项
/// 全局 D-Bus 串行锁只在短操作期间持有，所有等待都在锁外进行，
/// 否则切卡期间会阻塞其它所有 API 调用。
/// 另外，`auto_configure_apn` / `set_data_connection` 内部会自行加锁，
/// 因此调用它们时外面不能再套 `with_serial`，否则会死锁。
///
/// # Returns
/// 各阶段执行结果组成的描述字符串
pub async fn switch_sim_slot(conn: &Connection, slot: u8) -> zbus::Result<String> {
    let value = match slot {
        1 => SIM_SLOT_1_VALUE,
        2 => SIM_SLOT_2_VALUE,
        _ => return Err(zbus::Error::Failure("Invalid slot number, must be 1 or 2".to_string())),
    };

    let mut steps: Vec<String> = Vec::new();

    // ---- 阶段 1：拆连接 + modem 下线 + 发切卡指令（短操作，持锁串行化）----
    let (at_response, phase1_notes) = with_serial(async {
        let mut notes = Vec::new();

        // 1.1 拆掉当前 PDP 连接，避免旧卡的会话残留到新卡上
        if let Ok(context_path) = find_internet_context(conn).await {
            if let Ok(proxy) = ConnectionContextProxy::builder(conn)
                .path(context_path.as_str())?
                .build()
                .await
            {
                if proxy
                    .set_property("Active", zbus::zvariant::Value::Bool(false))
                    .await
                    .is_ok()
                {
                    notes.push(format!("deactivated {}", context_path));
                }
            }
        }

        // 1.2 让 modem 下线，强制它重新枚举 SIM
        let modem = ModemProxy::new(conn).await?;
        let _ = modem
            .set_property("Online", zbus::zvariant::Value::Bool(false))
            .await;

        // 1.3 发送切卡指令
        //     注意：zbus 生成的 ModemProxy 不暴露 call 方法，
        //     发 AT 指令必须用原生 Proxy（与 send_at_command 保持一致）
        let at_proxy = Proxy::new(conn, "org.ofono", "/ril_0", "org.ofono.Modem").await?;
        let cmd = format!("AT+SPCONFIGSIMSLOT={}", value);
        let response: String = at_proxy.call("SendAtcmd", &(cmd.as_str())).await?;

        Ok::<(String, Vec<String>), zbus::Error>((response, notes))
    })
    .await?;

    steps.extend(phase1_notes);

    // 1.4 校验 AT 响应。原来这里直接返回，modem 回 ERROR 也被当成切换成功
    if at_response_failed(&at_response) {
        // 尝试把 modem 恢复上线，避免切卡失败后 modem 一直处于离线状态
        let _ = with_serial(async {
            let modem = ModemProxy::new(conn).await?;
            modem.set_property("Online", zbus::zvariant::Value::Bool(true)).await
        })
        .await;

        return Err(zbus::Error::Failure(format!(
            "Modem rejected SIM slot switch (response: {})",
            at_response.trim()
        )));
    }
    steps.push(format!("slot {} command accepted", slot));

    // ---- 阶段 2：modem 上线并等待新卡就绪（等待在锁外进行）----
    tokio::time::sleep(Duration::from_secs(2)).await;

    if let Err(e) = with_serial(async {
        let modem = ModemProxy::new(conn).await?;
        modem.set_property("Online", zbus::zvariant::Value::Bool(true)).await
    })
    .await
    {
        steps.push(format!("failed to bring modem online: {}", e));
    } else {
        steps.push("modem back online".to_string());
    }

    // 2.1 等待 SIM 被识别
    match wait_for_sim_ready(conn, SIM_READY_TIMEOUT).await {
        Ok(()) => steps.push("SIM ready".to_string()),
        Err(e) => {
            warn!(error = %e, "SIM slot switch: SIM not ready in time");
            steps.push(format!("SIM not ready: {}", e));
        }
    }

    // 2.2 等待网络注册完成。注册不上不代表切卡失败（可能新卡无信号/未开通），
    //     所以只记录状态，不中断流程
    match wait_for_network_registered(conn, NETWORK_REGISTER_TIMEOUT).await {
        Ok(status) => steps.push(format!("network {}", status)),
        Err(e) => {
            warn!(error = %e, "SIM slot switch: network not registered in time");
            steps.push(format!("network not registered: {}", e));
        }
    }

    // ---- 阶段 3：按新卡运营商重配 APN ----
    // 注意：auto_configure_apn 内部会调用 set_apn_property 自行加串行锁，
    // 这里不能再包 with_serial，否则同一个 tokio Mutex 重入会死锁
    match find_internet_context(conn).await {
        Ok(context_path) => match auto_configure_apn(conn, &context_path).await {
            Ok(msg) => steps.push(msg),
            Err(e) => {
                // 境外卡或注册信息尚未就绪时无法推断 APN，保留原值并提示
                steps.push(format!("APN left unchanged: {}", e));
            }
        },
        Err(e) => steps.push(format!("APN not reconfigured: {}", e)),
    }

    // ---- 阶段 4：重新激活数据连接 ----
    match set_data_connection(conn, true).await {
        Ok(()) => steps.push("data connection activated".to_string()),
        Err(e) => {
            warn!(error = %e, "SIM slot switch: failed to activate data connection");
            steps.push(format!("activation failed: {}", e));
        }
    }

    let summary = steps.join("; ");
    info!(steps = %summary, "SIM slot switched");
    Ok(summary)
}

