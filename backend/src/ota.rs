/*
 * @Author: 1orz cloudorzi@gmail.com
 * @Date: 2025-12-10 09:19:05
 * @LastEditors: 1orz cloudorzi@gmail.com
 * @LastEditTime: 2025-12-13 12:46:12
 * @FilePath: /udx710-backend/backend/src/ota.rs
 * @Description:
 *
 * Copyright (c) 2025 by 1orz, All Rights Reserved.
 */
//! OTA 更新模块
//!
//! 处理 OTA 更新包的上传、验证和应用。
//!
//! 安全模型（修复 P0）：
//! 1. 解包采用 Rust 库逐条目提取，对每个成员的解析路径做 containment 校验，
//!    拒绝 `..` / 绝对路径 / 符号链接 / 硬链接逃出 staging 目录（防路径穿越 RCE）。
//! 2. 真实性不再依赖包内自带的 MD5（攻击者可控），改为 Ed25519 签名验签：
//!    签名公钥编译进固件（OTA_PUBKEY），私钥仅在构建服务器/CI 保管。
//!    验签消息为 `version|arch|binary_md5`，缺失或验签失败一律拒绝安装。

use crate::models::{OtaMeta, OtaStatusResponse, OtaUploadResponse, OtaValidation};
use base64::Engine;
use ed25519_dalek::Verifier;
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// OTA 相关路径
const OTA_STAGING_DIR: &str = "/tmp/ota_staging";
const OTA_BINARY_PATH: &str = "/home/root/udx710";
const OTA_WWW_PATH: &str = "/home/root/www";

/// Ed25519 验签公钥（编译进固件）。
/// 对应私钥仅在构建服务器 / CI 中保管，绝不可进入仓库或设备。
/// 任何缺少有效签名的 OTA 包都将被拒绝。
const OTA_PUBKEY: [u8; 32] = [
    150, 84, 175, 80, 54, 9, 48, 125, 117, 12, 134, 43, 31, 22, 140, 33, 110, 209, 56, 251, 18, 24, 76, 99,
    43, 175, 80, 47, 78, 92, 188, 31,
];

/// 当前版本信息（编译时注入）
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 获取当前 commit（从环境变量或默认值）
pub fn get_current_commit() -> String {
    option_env!("GIT_COMMIT").unwrap_or("unknown").to_string()
}

/// 获取 OTA 更新状态
pub fn get_ota_status() -> OtaStatusResponse {
    let pending_meta = read_pending_meta();

    OtaStatusResponse {
        current_version: CURRENT_VERSION.to_string(),
        current_commit: get_current_commit(),
        pending_update: pending_meta.is_some(),
        pending_meta,
    }
}

/// 读取待安装的更新元数据
fn read_pending_meta() -> Option<OtaMeta> {
    let meta_path = format!("{}/meta.json", OTA_STAGING_DIR);
    if let Ok(content) = fs::read_to_string(&meta_path) {
        serde_json::from_str(&content).ok()
    } else {
        None
    }
}

/// 将相对路径安全拼接进 base，拒绝任何会逃出 base 的路径（../、绝对路径、盘符前缀等）。
fn safe_join(base: &Path, rel: &Path) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => return None,
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// 设置提取后文件权限：二进制 udx710 为 755（可执行），其余为 644。
/// 不保留包内原始权限位，避免引入 setuid/setgid 等危险位。
fn set_extracted_perms(target: &Path, rel: &Path) -> Result<(), String> {
    let mode = if rel.file_name().map(|n| n == "udx710").unwrap_or(false) {
        0o755
    } else {
        0o644
    };
    fs::set_permissions(target, fs::Permissions::from_mode(mode))
        .map_err(|e| format!("chmod {:?}: {}", target, e))
}

/// 安全解压 tar.gz：逐条目提取，对每个成员路径做 containment 校验，拒绝符号/硬链接。
fn extract_tar_gz(data: &[u8]) -> Result<(), String> {
    let staging = Path::new(OTA_STAGING_DIR);
    fs::create_dir_all(staging).map_err(|e| format!("create staging: {}", e))?;

    let gz = flate2::read::GzDecoder::new(data);
    let mut archive = tar::Archive::new(gz);
    let entries = archive.entries().map_err(|e| format!("tar read: {}", e))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| format!("tar entry: {}", e))?;
        let rel = entry
            .path()
            .map_err(|e| format!("tar path: {}", e))?
            .into_owned();

        // 拒绝符号链接 / 硬链接：可被用于指向 staging 之外，造成写入逃逸。
        match entry.header().entry_type() {
            tar::EntryType::Symlink | tar::EntryType::Link => {
                return Err(format!("Refusing symlink/hardlink in OTA package: {:?}", rel));
            }
            _ => {}
        }

        let target = safe_join(staging, &rel)
            .ok_or_else(|| format!("Refusing unsafe tar path (escapes staging): {:?}", rel))?;

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("mkdir: {}", e))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("mkdir parent: {}", e))?;
            }
            let mut out = fs::File::create(&target).map_err(|e| format!("create: {}", e))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("copy: {}", e))?;
            set_extracted_perms(&target, &rel)?;
        }
    }
    Ok(())
}

/// 安全解压 zip：逐条目提取，对每个成员路径做 containment 校验，拒绝 Unix 符号链接。
fn extract_zip(data: &[u8]) -> Result<(), String> {
    let staging = Path::new(OTA_STAGING_DIR);
    fs::create_dir_all(staging).map_err(|e| format!("create staging: {}", e))?;

    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| format!("zip open: {}", e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("zip entry {}: {}", i, e))?;
        let name = file.name().to_string();
        let rel = Path::new(&name);

        let target = safe_join(staging, rel)
            .ok_or_else(|| format!("Refusing unsafe zip path (escapes staging): {}", name))?;

        // 拒绝 Unix 符号链接（zip 以 mode 0o120000 表示）。Windows 上无 unix_mode，跳过该检查。
        #[cfg(unix)]
        {
            if let Some(mode) = file.unix_mode() {
                if mode & 0o120000 == 0o120000 {
                    return Err(format!("Refusing symlink in OTA package: {}", name));
                }
            }
        }

        if file.is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("mkdir: {}", e))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("mkdir parent: {}", e))?;
            }
            let mut out = fs::File::create(&target).map_err(|e| format!("create: {}", e))?;
            std::io::copy(&mut file, &mut out).map_err(|e| format!("copy: {}", e))?;
            set_extracted_perms(&target, rel)?;
        }
    }
    Ok(())
}

/// 处理上传的 OTA 包（支持 tar.gz 和 zip 格式）
pub fn handle_ota_upload(data: &[u8]) -> Result<OtaUploadResponse, String> {
    // 清理并创建临时目录
    let _ = fs::remove_dir_all(OTA_STAGING_DIR);
    fs::create_dir_all(OTA_STAGING_DIR)
        .map_err(|e| format!("Failed to create staging dir: {}", e))?;

    // 自动检测文件格式并安全解包（逐个成员校验路径，防路径穿越）
    let is_zip = detect_zip_format(data);
    if is_zip {
        extract_zip(data)?;
    } else {
        extract_tar_gz(data)?;
    }

    // 读取 meta.json
    let meta_path = format!("{}/meta.json", OTA_STAGING_DIR);
    let meta_content =
        fs::read_to_string(&meta_path).map_err(|_| "meta.json not found in OTA package".to_string())?;

    let meta: OtaMeta = serde_json::from_str(&meta_content)
        .map_err(|e| format!("Invalid meta.json: {}", e))?;

    // 验证（含 Ed25519 验签）
    let validation = validate_ota_package(&meta)?;

    Ok(OtaUploadResponse { meta, validation })
}

/// 验签：对 `version|arch|binary_md5` 的规范串用编译进固件的公钥验签。
/// 返回 true 表示签名存在且有效。
fn verify_package_signature(meta: &OtaMeta, binary_md5: &str) -> bool {
    let sig_b64 = match &meta.signature {
        Some(s) if !s.is_empty() => s,
        _ => return false,
    };
    let sig_bytes = match base64::engine::general_purpose::STANDARD.decode(sig_b64) {
        Ok(b) => b,
        Err(_) => return false,
    };
    if sig_bytes.len() != 64 {
        return false;
    }
    let sig_arr: [u8; 64] = match sig_bytes.as_slice().try_into() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let vk = match ed25519_dalek::VerifyingKey::from_bytes(&OTA_PUBKEY) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
    let msg = format!("{}|{}|{}", meta.version, meta.arch, binary_md5);
    vk.verify(msg.as_bytes(), &sig).is_ok()
}

/// 验证 OTA 包
fn validate_ota_package(meta: &OtaMeta) -> Result<OtaValidation, String> {
    let binary_path = format!("{}/udx710", OTA_STAGING_DIR);
    let www_path = format!("{}/www", OTA_STAGING_DIR);

    // 检查文件存在
    if !Path::new(&binary_path).exists() {
        return Ok(OtaValidation {
            valid: false,
            is_newer: false,
            binary_md5_match: false,
            frontend_md5_match: false,
            arch_match: false,
            error: Some("Binary file not found in package".to_string()),
        });
    }

    if !Path::new(&www_path).exists() {
        return Ok(OtaValidation {
            valid: false,
            is_newer: false,
            binary_md5_match: false,
            frontend_md5_match: false,
            arch_match: false,
            error: Some("Frontend directory not found in package".to_string()),
        });
    }

    // 计算二进制 MD5（完整性校验，配合签名使用）
    let binary_md5 = calculate_file_md5(&binary_path)?;
    let binary_md5_match = binary_md5 == meta.binary_md5;

    // 前端目录存在即可（MD5 跨平台难以保持一致）
    let frontend_md5_match = true; // 跳过前端 MD5 验证

    // 检查架构（只接受 musl）
    let arch_match = meta.arch == "aarch64-unknown-linux-musl";

    // Ed25519 验签（核心真实性校验）
    let signature_ok = verify_package_signature(meta, &binary_md5);

    // 比较版本
    let is_newer = compare_versions(&meta.version, CURRENT_VERSION);

    // 只有通过签名 + 架构匹配才视为有效
    let valid = arch_match && signature_ok;

    // 生成详细的错误信息
    let error = if !valid {
        let mut errors = Vec::new();
        if !signature_ok {
            errors.push("Signature verification failed (missing or invalid Ed25519 signature)".to_string());
        }
        if !arch_match {
            errors.push(format!(
                "Arch mismatch: expected=aarch64-unknown-linux-musl, actual={}",
                meta.arch
            ));
        }
        if !binary_md5_match {
            errors.push(format!(
                "Binary MD5 mismatch: expected={}, actual={}",
                meta.binary_md5, binary_md5
            ));
        }
        Some(errors.join("; "))
    } else {
        None
    };

    Ok(OtaValidation {
        valid,
        is_newer,
        binary_md5_match,
        frontend_md5_match,
        arch_match,
        error,
    })
}

/// 计算文件 MD5
fn calculate_file_md5(path: &str) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;

    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|e| format!("Failed to read file: {}", e))?;

    Ok(format!("{:x}", md5::compute(&contents)))
}

/// 比较版本号（返回 v1 > v2）
fn compare_versions(v1: &str, v2: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };

    let v1_parts = parse(v1);
    let v2_parts = parse(v2);

    for i in 0..std::cmp::max(v1_parts.len(), v2_parts.len()) {
        let p1 = v1_parts.get(i).unwrap_or(&0);
        let p2 = v2_parts.get(i).unwrap_or(&0);
        if p1 > p2 {
            return true;
        } else if p1 < p2 {
            return false;
        }
    }
    false
}

/// 应用 OTA 更新
pub fn apply_ota_update(restart_now: bool) -> Result<String, String> {
    let meta = read_pending_meta().ok_or_else(|| "No pending update".to_string())?;

    let staging_binary = format!("{}/udx710", OTA_STAGING_DIR);
    let staging_www = format!("{}/www", OTA_STAGING_DIR);

    // 应用前强制重新校验：重新计算二进制 MD5 并验签，防止绕过上传期校验。
    // 也在此强制架构匹配与防降级。
    if !Path::new(&staging_binary).exists() {
        return Err("Staging binary missing; cannot apply".to_string());
    }
    let binary_md5 = calculate_file_md5(&staging_binary)?;
    if !verify_package_signature(&meta, &binary_md5) {
        return Err("OTA package signature verification failed; refusing to apply".to_string());
    }
    if meta.arch != "aarch64-unknown-linux-musl" {
        return Err(format!("OTA arch mismatch: {}", meta.arch));
    }
    if let Some(min) = &meta.min_version {
        if CURRENT_VERSION != min.as_str() && !compare_versions(CURRENT_VERSION, min) {
            return Err(format!(
                "Refusing downgrade: current {} is below min required {}",
                CURRENT_VERSION, min
            ));
        }
    }

    // 先将新二进制复制到临时路径，再用 rename 原子替换（避免 ETXTBSY）
    let tmp_binary = format!("{}.new", OTA_BINARY_PATH);
    fs::copy(&staging_binary, &tmp_binary)
        .map_err(|e| format!("Failed to copy binary to temp path: {}", e))?;
    // 设置权限
    Command::new("chmod")
        .args(["755", &tmp_binary])
        .output()
        .map_err(|e| format!("Failed to chmod: {}", e))?;
    // 原子替换：rename 对运行中文件是安全的
    fs::rename(&tmp_binary, OTA_BINARY_PATH)
        .map_err(|e| format!("Failed to replace binary: {}", e))?;

    // 复制前端文件（删除旧目录，复制新目录）
    let _ = fs::remove_dir_all(OTA_WWW_PATH);
    copy_dir_recursive(&staging_www, OTA_WWW_PATH)?;

    // 应用后统一修复权限（二进制 755 / www 目录 755 / 文件 644），并重建 loader 引导钩子
    fix_file_permissions("/home/root")?;
    crate::config::ensure_loader_hooks_init()?;

    // 清理暂存目录
    let _ = fs::remove_dir_all(OTA_STAGING_DIR);

    let message = format!("Update to version {} applied successfully", meta.version);

    if restart_now {
        // 延迟 1 秒后重启：先启动新进程（会等待端口释放），然后退出当前进程
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(1));
            // 启动新进程（新进程会轮询等待端口释放）
            let _ = Command::new("/home/root/udx710").args(["-p", "80"]).spawn();
            // 等待一小段时间确保新进程已启动
            std::thread::sleep(std::time::Duration::from_millis(200));
            // 退出当前进程，释放端口
            std::process::exit(0);
        });
    }

    Ok(message)
}

/// 递归复制目录
fn copy_dir_recursive(src: &str, dst: &str) -> Result<(), String> {
    fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create dir: {}", e))?;

    let entries = fs::read_dir(src).map_err(|e| format!("Failed to read src dir: {}", e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let src_path = entry.path();
        let dst_path = Path::new(dst).join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(
                src_path.to_str().unwrap_or(""),
                dst_path.to_str().unwrap_or(""),
            )?;
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Failed to copy file: {}", e))?;
        }
    }

    Ok(())
}

/// 取消待安装的更新
pub fn cancel_pending_update() -> Result<(), String> {
    if Path::new(OTA_STAGING_DIR).exists() {
        fs::remove_dir_all(OTA_STAGING_DIR)
            .map_err(|e| format!("Failed to remove staging dir: {}", e))?;
    }
    Ok(())
}

/// 检测文件是否为 ZIP 格式（通过魔术字节）
fn detect_zip_format(data: &[u8]) -> bool {
    // ZIP 文件魔术字节: PK\x03\x04 (0x504B0304)
    // TAR.GZ 文件魔术字节: \x1f\x8b (gzip header)
    if data.len() < 4 {
        return false;
    }

    // 检查是否是 ZIP 格式
    data[0] == 0x50 && data[1] == 0x4B && data[2] == 0x03 && data[3] == 0x04
}

/// 统一修复 OTA 相关文件权限（来自上游实现）：
/// 二进制 755，www 下目录 755、文件 644。
/// 解包时已按条目设置过权限，此函数用于应用后兜底（如复制过程丢失权限位）。
fn fix_file_permissions(root: &str) -> Result<(), String> {
    let binary_path = format!("{}/udx710", root);
    let www_path = format!("{}/www", root);

    if Path::new(&binary_path).exists() {
        Command::new("chmod")
            .args(["755", &binary_path])
            .output()
            .map_err(|e| format!("Failed to chmod binary {}: {}", binary_path, e))?;
    }

    if Path::new(&www_path).exists() {
        let _ = Command::new("find")
            .args([&www_path, "-type", "d", "-exec", "chmod", "755", "{}", "+"])
            .output();

        let _ = Command::new("find")
            .args([&www_path, "-type", "f", "-exec", "chmod", "644", "{}", "+"])
            .output();
    }

    Ok(())
}
