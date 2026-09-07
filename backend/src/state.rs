/*
 * @Author: 1orz cloudorzi@gmail.com
 * @Date: 2025-12-10 10:09:22
 * @LastEditors: 1orz cloudorzi@gmail.com
 * @LastEditTime: 2026-04-18 20:15:00
 * @FilePath: /udx710-backend/backend/src/state.rs
 * @Description:
 *
 * Copyright (c) 2025 by 1orz, All Rights Reserved.
 */

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::extract::FromRef;
use zbus::Connection;

use crate::config::ConfigManager;
use crate::db::Database;
use crate::scheduled_reboot::ScheduledRebootManager;
use crate::usage::DataUsageTracker;
use crate::sms_push::SmsPushSender;
use crate::webhook::WebhookSender;

pub struct FrontendRuntime {
    last_seen: RwLock<Option<Instant>>,
}

impl FrontendRuntime {
    pub fn new() -> Self {
        Self {
            last_seen: RwLock::new(None),
        }
    }

    pub fn mark_seen(&self) {
        *self.last_seen.write().unwrap() = Some(Instant::now());
    }

    pub fn is_recent(&self, timeout: Duration) -> bool {
        self.last_seen
            .read()
            .unwrap()
            .is_some_and(|last_seen| last_seen.elapsed() <= timeout)
    }
}

#[derive(Clone)]
pub struct AppState {
    pub dbus_conn: Arc<Connection>,
    pub database: Arc<Database>,
    pub config_manager: Arc<ConfigManager>,
    pub webhook_sender: Arc<WebhookSender>,
    /// 定时重启调度器
    pub scheduled_reboot_manager: Arc<ScheduledRebootManager>,
    /// 数据流量累计追踪器（流量限额功能）
    pub data_usage_tracker: Arc<DataUsageTracker>,
    /// 短信推送发送器（上游）
    pub sms_push_sender: Arc<SmsPushSender>,
    /// 前端在线状态（上游，自适应轮询用）
    pub frontend_runtime: Arc<FrontendRuntime>,
}

impl AppState {
    pub fn new(
        dbus_conn: Arc<Connection>,
        database: Arc<Database>,
        config_manager: Arc<ConfigManager>,
        webhook_sender: Arc<WebhookSender>,
        scheduled_reboot_manager: Arc<ScheduledRebootManager>,
        data_usage_tracker: Arc<DataUsageTracker>,
        sms_push_sender: Arc<SmsPushSender>,
        frontend_runtime: Arc<FrontendRuntime>,
    ) -> Self {
        Self {
            dbus_conn,
            database,
            config_manager,
            webhook_sender,
            scheduled_reboot_manager,
            data_usage_tracker,
            sms_push_sender,
            frontend_runtime,
        }
    }
}

impl FromRef<AppState> for Arc<Connection> {
    fn from_ref(state: &AppState) -> Self {
        state.dbus_conn.clone()
    }
}

impl FromRef<AppState> for Arc<Database> {
    fn from_ref(state: &AppState) -> Self {
        state.database.clone()
    }
}

impl FromRef<AppState> for Arc<ConfigManager> {
    fn from_ref(state: &AppState) -> Self {
        state.config_manager.clone()
    }
}

impl FromRef<AppState> for Arc<WebhookSender> {
    fn from_ref(state: &AppState) -> Self {
        state.webhook_sender.clone()
    }
}

impl FromRef<AppState> for Arc<ScheduledRebootManager> {
    fn from_ref(state: &AppState) -> Self {
        state.scheduled_reboot_manager.clone()
    }
}

impl FromRef<AppState> for Arc<DataUsageTracker> {
    fn from_ref(state: &AppState) -> Self {
        state.data_usage_tracker.clone()
    }
}

impl FromRef<AppState> for Arc<SmsPushSender> {
    fn from_ref(state: &AppState) -> Self {
        state.sms_push_sender.clone()
    }
}

impl FromRef<AppState> for Arc<FrontendRuntime> {
    fn from_ref(state: &AppState) -> Self {
        state.frontend_runtime.clone()
    }
}

// 支持 (Arc<Connection>, Arc<Database>) 元组类型
impl FromRef<AppState> for (Arc<Connection>, Arc<Database>) {
    fn from_ref(state: &AppState) -> Self {
        (state.dbus_conn.clone(), state.database.clone())
    }
}
