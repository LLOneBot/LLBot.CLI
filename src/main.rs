//! LLBot CLI - 启动器

mod pmhq_client;
mod qrcode_display;

use pmhq_client::PMHQClient;
use qrcode_display::{print_qrcode_terminal, save_qrcode_image};
use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const DEFAULT_PORT: u16 = 13000;
const PORT_RANGE_END: u16 = 14000;

fn should_show_terminal_qrcode(exe_dir: &Path, args: &[String]) -> bool {
    if cfg!(not(target_os = "windows")) {
        return true;
    }
    
    if args.iter().any(|a| a == "--headless") {
        return true;
    }
    
    let config_path = exe_dir.join("bin/pmhq/pmhq_config.json");
    if let Ok(content) = fs::read_to_string(&config_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            return json.get("headless").and_then(|v| v.as_bool()).unwrap_or(false);
        }
    }
    false
}

fn get_exe_name(base: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", base)
    } else {
        base.to_string()
    }
}

fn main() {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let args: Vec<String> = env::args().skip(1).collect();
    let pmhq_exe = exe_dir.join(format!("bin/pmhq/{}", get_exe_name("pmhq")));

    // --help 或 --version 直接转发给 pmhq
    if args.iter().any(|a| a == "--help" || a == "-h" || a == "--version") {
        if pmhq_exe.exists() {
            let status = Command::new(&pmhq_exe).args(&args).status();
            std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
        } else {
            eprintln!("错误: 未找到 pmhq: {}", pmhq_exe.display());
            std::process::exit(1);
        }
    }

    migrate_old_files(&exe_dir);

    let llbot_dir = exe_dir.join("bin/llbot");
    let node_exe = get_exe_name("node");

    if !pmhq_exe.exists() {
        eprintln!("错误: 未找到 pmhq: {}", pmhq_exe.display());
        wait_exit(1);
    }
    if !llbot_dir.join(&node_exe).exists() {
        eprintln!(
            "错误: 未找到 {}: {}",
            node_exe,
            llbot_dir.join(&node_exe).display()
        );
        wait_exit(1);
    }
    if !llbot_dir.join("llbot.js").exists() {
        eprintln!(
            "错误: 未找到 llbot.js: {}",
            llbot_dir.join("llbot.js").display()
        );
        wait_exit(1);
    }

    let port = find_available_port(DEFAULT_PORT, PORT_RANGE_END).unwrap_or_else(|| {
        eprintln!("错误: 无法找到可用端口 ({}-{})", DEFAULT_PORT, PORT_RANGE_END);
        wait_exit(1);
    });

    println!("LLBot CLI 启动器");
    println!("================");
    println!("端口: {}", port);
    println!();

    let mut cmd = Command::new(&pmhq_exe);
    cmd.arg("--port").arg(port.to_string());
    
    if !args.is_empty() {
        cmd.args(&args);
    }
    
    cmd.arg("--sub-cmd-workdir")
        .arg(&llbot_dir)
        .arg("--sub-cmd")
        .arg(&node_exe)
        .arg("--enable-source-maps")
        .arg("llbot.js")
        .arg("--")
        .arg(format!("--pmhq-port={}", port));

    let mut child = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("启动 pmhq 失败: {}", e);
            wait_exit(1);
        }
    };

    let child_id = child.id();
    let qq_pid_cache = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let qq_pid_for_cleanup = qq_pid_cache.clone();
    
    ctrlc::set_handler(move || {
        let cached_pid = qq_pid_for_cleanup.load(Ordering::Relaxed);
        cleanup_and_exit(child_id, if cached_pid > 0 { Some(cached_pid) } else { None });
    })
    .ok();

    // 读取 pmhq stdout，解析 QQ PID
    let qq_pid_from_stdout = qq_pid_cache.clone();
    if let Some(stdout) = child.stdout.take() {
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("{}", line);
                    // 解析 "QQ 进程 PID: 12345" 或 "QQ进程PID: 12345"
                    if line.contains("PID:") && line.contains("QQ") {
                        if let Some(pos) = line.rfind("PID:") {
                            let after_pid = &line[pos + 4..];
                            let pid_str: String = after_pid.chars()
                                .skip_while(|c| c.is_whitespace())
                                .take_while(|c| c.is_ascii_digit())
                                .collect();
                            if let Ok(pid) = pid_str.parse::<u32>() {
                                qq_pid_from_stdout.store(pid, Ordering::Relaxed);
                            }
                        }
                    }
                }
            }
        });
    }

    // 读取 pmhq stderr
    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(line) = line {
                    eprintln!("{}", line);
                }
            }
        });
    }

    let logged_in = Arc::new(AtomicBool::new(false));
    let qrcode_path = exe_dir.join("qrcode.png");
    let show_terminal_qr = should_show_terminal_qrcode(&exe_dir, &args);

    start_login_listener(port, logged_in.clone(), qrcode_path, show_terminal_qr);

    match child.wait() {
        Ok(status) => {
            if !status.success() {
                eprintln!("pmhq 退出，状态码: {:?}", status.code());
            }
        }
        Err(e) => {
            eprintln!("等待 pmhq 失败: {}", e);
        }
    }
}

fn start_login_listener(
    port: u16,
    logged_in: Arc<AtomicBool>,
    qrcode_path: PathBuf,
    show_terminal_qr: bool,
) {
    thread::spawn(move || {
        let client = PMHQClient::new(port).with_timeout(Duration::from_secs(10));

        thread::sleep(Duration::from_secs(3));

        let logged_in_refresh = logged_in.clone();
        let client_refresh = client.clone();
        thread::spawn(move || {
            loop {
                if logged_in_refresh.load(Ordering::Relaxed) {
                    break;
                }
                let _ = client_refresh.request_qrcode();
                for _ in 0..120 {
                    if logged_in_refresh.load(Ordering::Relaxed) {
                        break;
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        });

        client.start_sse_listener(logged_in.clone(), move |qrcode_url, png_base64| {
            if show_terminal_qr {
                print_qrcode_terminal(qrcode_url);
            }

            if !png_base64.is_empty() {
                if let Err(e) = save_qrcode_image(png_base64, &qrcode_path) {
                    eprintln!("保存二维码失败: {}", e);
                } else {
                    println!("二维码文件: {}", qrcode_path.display());
                }
            }

            println!(
                "二维码网址: https://api.2dcode.biz/v1/create-qr-code?data={}",
                qrcode_url
            );
            println!("请使用手机QQ扫码登录");
            println!();
        });

        if logged_in.load(Ordering::Relaxed) {
            println!();
            println!("================");
            println!("登录成功!");

            if let Ok(info) = client.get_self_info() {
                println!("QQ号: {}", info.uin);
                if !info.nickname.is_empty() {
                    println!("昵称: {}", info.nickname);
                }
            }
            println!("================");
            println!();
        }
    });
}

fn migrate_old_files(exe_dir: &Path) {
    // 迁移 data 目录
    let data_dir = exe_dir.join("data");
    let target_data_dir = exe_dir.join("bin/llbot/data");
    if data_dir.exists() && data_dir.is_dir() {
        println!("检测到 data 目录，正在移动到 bin/llbot/...");
        if target_data_dir.exists() {
            let _ = fs::remove_dir_all(&target_data_dir);
        }
        if fs::rename(&data_dir, &target_data_dir).is_err() {
            if let Err(e) = copy_dir_recursive(&data_dir, &target_data_dir) {
                eprintln!("警告: 移动 data 目录失败: {}", e);
            } else {
                let _ = fs::remove_dir_all(&data_dir);
                println!("data 目录移动完成");
            }
        } else {
            println!("data 目录移动完成");
        }
    }

    // 迁移 pmhq_config.json
    let pmhq_config = exe_dir.join("pmhq_config.json");
    let target_pmhq_config = exe_dir.join("bin/pmhq/pmhq_config.json");
    if pmhq_config.exists() && pmhq_config.is_file() {
        println!("检测到 pmhq_config.json，正在移动到 bin/pmhq/...");
        if target_pmhq_config.exists() {
            let _ = fs::remove_file(&target_pmhq_config);
        }
        if fs::rename(&pmhq_config, &target_pmhq_config).is_err() {
            if let Err(e) = fs::copy(&pmhq_config, &target_pmhq_config) {
                eprintln!("警告: 移动 pmhq_config.json 失败: {}", e);
            } else {
                let _ = fs::remove_file(&pmhq_config);
                println!("pmhq_config.json 移动完成");
            }
        } else {
            println!("pmhq_config.json 移动完成");
        }
    }
}

fn find_available_port(start: u16, end: u16) -> Option<u16> {
    for port in start..end {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

fn wait_exit(code: i32) -> ! {
    println!("\n按 Enter 键退出...");
    let mut input = String::new();
    let _ = std::io::stdin().read_line(&mut input);
    std::process::exit(code);
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !dst.exists() {
        fs::create_dir_all(dst)?;
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn kill_process_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(target_os = "windows"))]
fn kill_process_tree(pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{}", pid)])
        .status();
}

fn cleanup_and_exit(pmhq_pid: u32, qq_pid: Option<u32>) {
    // 先杀 QQ，再杀 pmhq（因为 taskkill /T 会终止整个进程树）
    if let Some(pid) = qq_pid {
        kill_process(pid);
    }
    
    kill_process_tree(pmhq_pid);
    std::process::exit(0);
}

#[cfg(target_os = "windows")]
fn kill_process(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(target_os = "windows"))]
fn kill_process(pid: u32) {
    let _ = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status();
}
