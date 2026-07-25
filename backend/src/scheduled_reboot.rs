//! 定时重启模块
//!
//! 根据配置在指定时间点自动重启系统。
//! 支持配置：启用开关、间隔天数、小时、分钟。

use crate::config::{ConfigManager, ScheduledRebootConfig};
use chrono::{Local, NaiveTime, Duration};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tracing::{info, warn};

/// 定时重启调度器
pub struct ScheduledRebootManager {
    #[allow(dead_code)]
    config_manager: Arc<ConfigManager>,
    /// 通知调度器重新计算休眠时间
    notify: Arc<Notify>,
    /// 保护状态，防止重复重启
    #[allow(dead_code)]
    state: Arc<Mutex<SchedulerState>>,
}

struct SchedulerState {
    /// 是否正在等待重启
    waiting: bool,
}

impl ScheduledRebootManager {
    /// 创建并启动定时重启调度器
    pub fn new(config_manager: Arc<ConfigManager>) -> Self {
        let notify = Arc::new(Notify::new());
        let state = Arc::new(Mutex::new(SchedulerState { waiting: false }));

        // 启动后台调度任务
        tokio::spawn(schedule_loop(config_manager.clone(), notify.clone(), state.clone()));

        Self {
            config_manager,
            notify,
            state,
        }
    }

    /// 唤醒调度器（当配置变更时调用，重新计算下次重启时间）
    pub fn wake(&self) {
        info!("Scheduled reboot: waking scheduler, recalibrating...");
        self.notify.notify_one();
    }
}

/// 后台调度循环：计算下次重启时间 → sleep → 执行 → 循环
async fn schedule_loop(
    config_manager: Arc<ConfigManager>,
    notify: Arc<Notify>,
    state: Arc<Mutex<SchedulerState>>,
) {
    loop {
        let reboot_config = config_manager.get_scheduled_reboot();

        if !reboot_config.enabled {
            info!("Scheduled reboot: disabled, waiting for config change...");
            notify.notified().await;
            continue;
        }

        // 计算到下次重启还有多少秒
        let sleep_seconds = match calc_seconds_until_next_reboot(&reboot_config) {
            Some(secs) => secs,
            None => {
                warn!("Scheduled reboot: failed to calculate next reboot time, retrying in 60s");
                60
            }
        };

        info!(
            "Scheduled reboot: next reboot in {} seconds ({:.0} hours), at {:04}:{:02} (interval: {} day(s))",
            sleep_seconds,
            sleep_seconds as f64 / 3600.0,
            reboot_config.hour,
            reboot_config.minute,
            reboot_config.interval_days,
        );

        // 用 notify 实现可中断 sleep
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(sleep_seconds)) => {
                // 时间到了，执行重启
                // 再次检查配置是否仍然启用（防止在 sleep 期间被关闭）
                let current_config = config_manager.get_scheduled_reboot();
                if !current_config.enabled {
                    info!("Scheduled reboot: disabled during wait, skipping reboot");
                    continue;
                }

                {
                    let mut s = state.lock().await;
                    s.waiting = true;
                }

                info!("Scheduled reboot: executing reboot...");
                let _ = tokio::process::Command::new("reboot").output().await;
            }
            _ = notify.notified() => {
                // 配置变更，重新循环计算
                info!("Scheduled reboot: config changed, recalibrating...");
                continue;
            }
        }
    }
}

/// 计算距离下一次目标时间点的秒数
///
/// 逻辑：
/// 1. 当前时间
/// 2. 计算今天的目标时间点（hour:minute）
/// 3. 如果今天的目标时间已过，看 interval_days 决定推到明天还是后几天
/// 4. 返回 sleep 秒数
fn calc_seconds_until_next_reboot(config: &ScheduledRebootConfig) -> Option<u64> {
    let now = Local::now().naive_local();
    let today_target = NaiveTime::from_hms_opt(config.hour as u32, config.minute as u32, 0)?;

    let today_target_dt = now.date().and_time(today_target);

    // 如果今天目标时间还没到
    if now < today_target_dt {
        let diff = today_target_dt - now;
        return Some(diff.num_seconds().max(0) as u64);
    }

    // 今天目标时间已过，算明天的目标时间
    // 如果 interval_days > 1，需要跳过更多天
    let days_to_add = if config.interval_days <= 1 {
        1
    } else {
        // 每 interval_days 天重启一次，从 epoch 基准对齐
        // 用今天日期距 epoch 的天数来对齐
        let today_abs = now.date();
        // 计算从 epoch (1970-01-01) 到今天的秒数
        let epoch_seconds = today_abs
            .and_hms_opt(0, 0, 0)?
            .and_utc()
            .timestamp();
        let days_since_epoch = epoch_seconds / 86400;
        // 下次重启 = 向上取整到下一个 interval_days 的倍数
        let next_epoch_day = ((days_since_epoch / config.interval_days as i64) + 1)
            * config.interval_days as i64;
        let days_from_today = (next_epoch_day - days_since_epoch).max(1) as i64;
        days_from_today as i64
    };

    // 使用 chrono::Duration 安全计算
    let tomorrow = now.date() + Duration::days(days_to_add);
    let next_target = tomorrow.and_time(today_target);
    let diff = next_target - now;
    Some(diff.num_seconds().max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calc_future_time_today() {
        // 时间逻辑依赖当前时间，仅做基本烟雾测试
        let config = ScheduledRebootConfig {
            enabled: true,
            interval_days: 1,
            hour: 23,
            minute: 59,
        };
        let secs = calc_seconds_until_next_reboot(&config);
        assert!(secs.is_some());
        // 至少 > 0（除非现在是 23:59）
    }

    #[test]
    fn test_calc_multi_day_interval() {
        let config = ScheduledRebootConfig {
            enabled: true,
            interval_days: 3,
            hour: 4,
            minute: 0,
        };
        let secs = calc_seconds_until_next_reboot(&config);
        assert!(secs.is_some());
        assert!(secs.unwrap() > 0);
    }
}
