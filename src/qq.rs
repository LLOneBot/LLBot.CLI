use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use crate::util;

const QQ_DOWNLOAD_URL: &str =
    "https://dldir1v6.qq.com/qqfile/qq/QQNT/c50d6326/QQ9.9.22.40768_x64.exe";

pub fn detect_qq_path(exe_dir: &Path, args: &[String]) -> Option<String> {
    if !cfg!(any(target_os = "windows", target_os = "macos")) {
        return None;
    }

    let qq_path_arg = args
        .iter()
        .find(|a| a.starts_with("--qq-path="))
        .map(|a| a.trim_start_matches("--qq-path=").to_string());

    let qq_path_arg_invalid = qq_path_arg
        .as_ref()
        .map(|p| !Path::new(p).exists())
        .unwrap_or(false);

    if qq_path_arg_invalid {
        eprintln!(
            "错误: 指定的 QQ 路径不存在: {}",
            qq_path_arg.as_ref().unwrap()
        );
    }

    let qq_path = if qq_path_arg_invalid {
        None
    } else {
        qq_path_arg.or_else(|| get_qq_path_from_registry(exe_dir))
    };

    if qq_path.is_none() || !qq_path.as_ref().map(|p| Path::new(p).exists()).unwrap_or(false) {
        if cfg!(target_os = "windows") {
            println!("未找到 QQ，是否下载并安装？(y/n)");
            let mut input = String::new();
            if std::io::stdin().read_line(&mut input).is_ok() {
                if input.trim().eq_ignore_ascii_case("y") {
                    if !download_and_install_qq() {
                        eprintln!("QQ 下载安装失败");
                        util::wait_exit(1);
                    }
                    println!("QQ 安装完成，请重新运行程序");
                    util::wait_exit(0);
                } else {
                    eprintln!("错误: 未找到 QQ，请安装 QQ 或使用 --qq-path 参数指定路径");
                    util::wait_exit(1);
                }
            }
        } else {
            eprintln!("错误: 未找到 QQ，请安装 QQ 或使用 --qq-path 参数指定路径");
            eprintln!("提示: 请将 QQ 安装到 /Applications/QQ.app 或放置到 bin/qq/QQ.app");
            util::wait_exit(1);
        }
    }

    qq_path
}

#[cfg(target_os = "windows")]
fn get_qq_path_from_registry(_exe_dir: &Path) -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey(r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\QQ")
        .ok()?;

    let uninstall_path: String = key.get_value("UninstallString").ok()?;
    let uninstall_path = uninstall_path.trim_matches('"');

    let qq_dir = Path::new(uninstall_path).parent()?;
    let qq_exe = qq_dir.join("QQ.exe");

    if qq_exe.exists() {
        Some(qq_exe.to_string_lossy().to_string())
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn get_qq_path_from_registry(exe_dir: &Path) -> Option<String> {
    // 优先检查当前目录的 bin/qq/QQ.app
    let local_qq = exe_dir.join("bin/qq/QQ.app/Contents/MacOS/QQ");
    if local_qq.exists() {
        return Some(local_qq.to_string_lossy().to_string());
    }

    // 其次检查系统 Applications 目录
    let system_qq = Path::new("/Applications/QQ.app/Contents/MacOS/QQ");
    if system_qq.exists() {
        return Some(system_qq.to_string_lossy().to_string());
    }

    None
}

#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
fn get_qq_path_from_registry(_exe_dir: &Path) -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
fn download_and_install_qq() -> bool {
    println!("正在下载 QQ...");

    let temp_dir = env::temp_dir();
    let temp_file = temp_dir.join("QQ_Setup.exe");

    match ureq::get(QQ_DOWNLOAD_URL)
        .timeout(std::time::Duration::from_secs(300))
        .call()
    {
        Ok(resp) => {
            let total_size = resp
                .header("Content-Length")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            let mut file = match File::create(&temp_file) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("创建临时文件失败: {}", e);
                    return false;
                }
            };

            let mut reader = resp.into_reader();
            let mut buffer = [0u8; 65536];
            let mut downloaded: u64 = 0;

            loop {
                match std::io::Read::read(&mut reader, &mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if file.write_all(&buffer[..n]).is_err() {
                            eprintln!("写入文件失败");
                            return false;
                        }
                        downloaded += n as u64;
                        if total_size > 0 {
                            print!(
                                "\r下载进度: {:.1} MB / {:.1} MB ({:.0}%)",
                                downloaded as f64 / 1024.0 / 1024.0,
                                total_size as f64 / 1024.0 / 1024.0,
                                downloaded as f64 / total_size as f64 * 100.0
                            );
                            let _ = std::io::stdout().flush();
                        }
                    }
                    Err(e) => {
                        eprintln!("\n下载失败: {}", e);
                        return false;
                    }
                }
            }
            println!();

            println!("正在安装 QQ（静默安装）...");
            match std::process::Command::new(&temp_file).arg("/S").status() {
                Ok(status) => {
                    let _ = fs::remove_file(&temp_file);
                    status.success()
                }
                Err(e) => {
                    eprintln!("启动安装程序失败: {}", e);
                    let _ = fs::remove_file(&temp_file);
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("下载失败: {}", e);
            false
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn download_and_install_qq() -> bool {
    eprintln!("QQ 自动安装仅支持 Windows");
    false
}

/// 检测 QQ 进程是否正在运行
#[cfg(target_os = "windows")]
pub fn is_qq_running() -> bool {
    use std::process::Command;

    match Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq QQ.exe", "/NH"])
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.contains("QQ.exe")
        }
        Err(_) => false,
    }
}

#[cfg(target_os = "macos")]
pub fn is_qq_running() -> bool {
    use std::process::Command;

    match Command::new("pgrep").args(["-x", "QQ"]).output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
pub fn is_qq_running() -> bool {
    use std::process::Command;

    // Linux 上尝试检测 QQ 相关进程
    match Command::new("pgrep").args(["-f", "qq"]).output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub fn is_qq_running() -> bool {
    // 其他平台默认返回 true，不触发退出
    true
}
