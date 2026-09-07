/*
 * @FilePath: /udx710-backend/backend/src/usage.rs
 * @Description: 数据流量累计追踪（持久化，跨重启保留）
 *
 * 内核 /sys/class/net/<iface>/statistics/rx_bytes 只统计"自接口 up 以来"的字节数，
 * 重启即清零。本模块额外维护一个持久化的累计值，每轮采样把接口的增量累加进去，
 * 从而提供"已用多少流量"的口径，供流量限额功能使用。
 */
//! 数据流量累计追踪模块
//!
//! 持久化累计蜂窝数据流量（接口 usb0），跨重启保留；提供限额阻断状态管理。

use crate::utils::read_interface_stats;
use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

/// 蜂窝数据接口（USB 网卡，运营商流量走这里）
const CELLULAR_INTERFACE: &str = "usb0";
/// 1 GB = 10^9 字节（与运营商计费口径一致，十进制）
pub const BYTES_PER_GB: u64 = 1_000_000_000;

/// 系统时间早于该年份视为"时钟未同步"。
///
/// 设备无 RTC 电池，断电重启后系统时间会回到 1970-01-01（日期=1 号），
/// 若此时执行按日清零，会把上个月的累计流量误清掉（默认清零日恰好是 1 号）。
/// 因此时钟未同步（早于项目时代）期间绝不自动清零，等 NTP 同步后再正常判断。
const MIN_PLAUSIBLE_YEAR: i32 = 2025;

/// 持久化状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DataUsageState {
    /// 累计接收字节数
    total_rx: u64,
    /// 累计发送字节数
    total_tx: u64,
    /// 上次采样时的接口 rx（用于计算增量 / 检测计数器回退）
    last_rx: u64,
    /// 上次采样时的接口 tx
    last_tx: u64,
    /// 是否已因到达流量限额被阻断（数据连接被强制关闭）
    blocked_by_limit: bool,
    /// 上次自动清零日期（本地时区，格式 YYYY-MM-DD）。None 表示从未自动清零。
    last_reset_date: Option<String>,
}

/// 计算某年某月的天数（用于把 reset_day 钳制到当月实际最大天数，
/// 例如选 31 但在 2 月（28/29 天）则按当月最后一天清零）。
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// 数据流量追踪器
pub struct DataUsageTracker {
    state: Mutex<DataUsageState>,
    path: PathBuf,
}

impl DataUsageTracker {
    /// 创建追踪器；若已存在持久化文件则加载，否则从零开始。
    ///
    /// 恢复顺序：主文件 → 同名 `.tmp` 临时文件（rename 前断电遗留的较新快照）→ 从零。
    /// 主文件损坏时先把坏文件改名备份（`.json.corrupt`）再尝试 tmp 恢复，保留现场便于排查。
    pub fn new(path: PathBuf) -> Self {
        let tmp_path = path.with_file_name(format!(
            "{}.tmp",
            path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        ));
        let loaded = fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_json::from_str::<DataUsageState>(&c).ok());
        let state = match loaded {
            Some(s) => s,
            None => {
                if path.exists() {
                    let backup = path.with_extension("json.corrupt");
                    match fs::rename(&path, &backup) {
                        Ok(_) => warn!(
                            path = %path.display(),
                            backup = %backup.display(),
                            "data_usage.json 解析失败（可能断电损坏），已备份并尝试从 .tmp 恢复"
                        ),
                        Err(e) => warn!(
                            path = %path.display(),
                            error = %e,
                            "data_usage.json 解析失败且备份失败，将从零累计"
                        ),
                    }
                }
                // 主文件缺失/损坏：尝试遗留的 .tmp（写盘成功但 rename 前断电的产物）
                let recovered = fs::read_to_string(&tmp_path)
                    .ok()
                    .and_then(|c| serde_json::from_str::<DataUsageState>(&c).ok());
                match recovered {
                    Some(s) => {
                        info!(path = %tmp_path.display(), "从遗留 .tmp 临时文件恢复流量累计");
                        s
                    }
                    None => DataUsageState::default(),
                }
            }
        };
        Self {
            state: Mutex::new(state),
            path,
        }
    }

    /// 持久化到磁盘（仅在状态变化时调用，避免无谓写盘）。
    ///
    /// 使用带 fsync 的原子写（见 utils::atomic_write_sync）：
    /// 断电时要么是旧文件、要么是完整新文件，不会出现截断的半截 JSON。
    fn persist(&self) {
        let Ok(s) = serde_json::to_string_pretty(&*self.state.lock().unwrap()) else {
            return;
        };
        if let Err(e) = crate::utils::atomic_write_sync(&self.path, &s) {
            warn!(path = %self.path.display(), error = %e, "data_usage.json 写盘失败");
        }
    }

    /// 采样一次：读取 usb0 当前 rx/tx，把相对上次的增量累加到持久化总量。
    /// 返回当前累计 (total_rx, total_tx)。
    ///
    /// 计数器回退（接口 reset / 设备重启导致 rx 变小）会被当作"重新基线"，
    /// 不计入负增量；首次采样（last=0）也不把历史全量计入。
    pub fn sample(&self) -> (u64, u64) {
        let (rx, tx) = match read_interface_stats(CELLULAR_INTERFACE) {
            Ok(v) => v,
            Err(_) => {
                let s = self.state.lock().unwrap();
                return (s.total_rx, s.total_tx);
            }
        };

        let mut st = self.state.lock().unwrap();
        let d_rx = rx.saturating_sub(st.last_rx);
        let d_tx = tx.saturating_sub(st.last_tx);
        // 仅在已有基线（last>0）时累加，避免首启把历史全量误计入
        if st.last_rx > 0 {
            st.total_rx = st.total_rx.saturating_add(d_rx);
            st.total_tx = st.total_tx.saturating_add(d_tx);
        }
        st.last_rx = rx;
        st.last_tx = tx;
        let (tr, tt) = (st.total_rx, st.total_tx);
        drop(st);
        self.persist();
        (tr, tt)
    }

    /// 当前累计 (total_rx, total_tx)
    pub fn get_usage(&self) -> (u64, u64) {
        let s = self.state.lock().unwrap();
        (s.total_rx, s.total_tx)
    }

    /// 是否已因限额被阻断
    pub fn is_blocked(&self) -> bool {
        self.state.lock().unwrap().blocked_by_limit
    }

    /// 设置阻断状态（仅在值变化时落盘）
    pub fn set_blocked(&self, blocked: bool) {
        let changed = {
            let mut s = self.state.lock().unwrap();
            if s.blocked_by_limit != blocked {
                s.blocked_by_limit = blocked;
                true
            } else {
                false
            }
        };
        if changed {
            self.persist();
        }
    }

    /// 清零累计流量并解除阻断（供"重置统计"使用）
    pub fn reset(&self) {
        {
            let mut s = self.state.lock().unwrap();
            *s = DataUsageState::default();
        }
        self.persist();
    }

    /// 读取上次自动清零日期（供前端展示）
    pub fn get_last_reset_date(&self) -> Option<String> {
        self.state.lock().unwrap().last_reset_date.clone()
    }

    /// 按设定的"重置日"自动清零（与是否设置限额无关）。
    ///
    /// 行为：仅当"今天是 `reset_day` 且该日尚未清零"时执行——清零累计收发字节、
    /// 解除限额阻断（新计费周期开始，恢复数据服务），并记录 `last_reset_date` 防同日重复清零。
    ///
    /// 边界：
    /// - 若设的值超过当月实际天数（如 2 月选 31），按当月最后一天清零，
    ///   这样选 31 在短月也能在月末清零（与"月底清零"意图一致）；
    /// - **时钟未同步（系统时间早于 `MIN_PLAUSIBLE_YEAR`）时绝不执行**——
    ///   设备无 RTC 电池，断电重启后时间回到 1970-01-01（日期=1 号），
    ///   若不设防会在每次断电重启时把累计流量误清零（历史 bug）。
    pub fn maybe_auto_reset(&self, reset_day: u8) {
        let now = Local::now();

        // 时钟未同步保护：1970 等早期时间的"日期"不可信，跳过清零。
        // NTP 同步后（同一天内或之后的 watchdog 轮询）会正常执行真实的按日清零。
        if now.year() < MIN_PLAUSIBLE_YEAR {
            warn!(
                year = now.year(),
                min_plausible_year = MIN_PLAUSIBLE_YEAR,
                "系统时钟疑似未同步，跳过流量自动清零以防断电重启后误清累计"
            );
            return;
        }

        let days_in_month = days_in_month(now.year(), now.month());
        let target = (reset_day.clamp(1, 31) as u32).min(days_in_month);
        if now.day() != target {
            return;
        }

        let today = now.format("%Y-%m-%d").to_string();
        {
            let s = self.state.lock().unwrap();
            if s.last_reset_date.as_deref() == Some(today.as_str()) {
                return; // 今天已清零，避免重复
            }
        }

        {
            let mut s = self.state.lock().unwrap();
            s.total_rx = 0;
            s.total_tx = 0;
            s.blocked_by_limit = false; // 新计费周期：解除限额阻断，允许恢复数据
            s.last_reset_date = Some(today.clone());
        }
        self.persist();
        info!(
            reset_day = target,
            date = %today,
            "流量按设定日自动清零（新计费周期开始）"
        );
    }
}
