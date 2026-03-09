//! LLBot CLI - 启动器

mod pmhq_client;
mod qrcode_display;
mod updater;
mod login;
mod migrate;
mod qq;
mod util;
mod windows_job;

use command_group::{CommandGroup, GroupChild};
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const DEFAULT_PORT: u16 = 13000;
const PORT_RANGE_END: u16 = 14000;
const DEFAULT_QQ_EXIT_DELAY: u64 = 15;

fn main() {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let args: Vec<String> = env::args().skip(1).collect();

    // --update 检查并执行更新（不依赖 pmhq 已安装）
    if args.iter().any(|a| a == "--update") {
        updater::run_update(&exe_dir);
        util::wait_exit(0);
    }

    // 启动时自动补齐必要组件（缺失才下载）
    if let Err(e) = updater::ensure_required_components(&exe_dir) {
        eprintln!("自动安装组件失败: {}", e);
        util::wait_exit(1);
    }

    let pmhq_exe = match util::find_pmhq_exe(&exe_dir) {
        Some(path) => path,
        None => {
            eprintln!("错误: 未找到 pmhq 可执行文件");
            eprintln!("请确保 bin/pmhq/ 目录下存在 pmhq 或 pmhq-<platform>-<arch> 文件");
            util::wait_exit(1);
        }
    };

    // --help 先输出 CLI 专用参数，再转发给 pmhq
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("llbot-cli {}", env!("CARGO_PKG_VERSION"));
        println!();
        println!("CLI 专用参数:");
        println!("  --no-exit-with-qq       禁用 QQ 退出时自动退出（默认启用）");
        println!("  --qq-exit-delay=<秒>    QQ 退出后的缓冲时间（默认 15 秒，0 为立即退出）");
        println!();
        let status = Command::new(&pmhq_exe).args(&args).status();
        std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
    }

    // --version 先输出 CLI 版本，再转发给 pmhq
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("llbot-cli {}", env!("CARGO_PKG_VERSION"));
        let status = Command::new(&pmhq_exe).args(&args).status();
        std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
    }

    // 解析 --exit-with-qq / --no-exit-with-qq 参数（默认启用）
    let exit_with_qq = !args.iter().any(|a| a == "--no-exit-with-qq");

    // 解析 --qq-exit-delay 参数（默认 15 秒）
    let qq_exit_delay: u64 = args
        .iter()
        .find(|a| a.starts_with("--qq-exit-delay="))
        .and_then(|a| a.trim_start_matches("--qq-exit-delay=").parse().ok())
        .unwrap_or(DEFAULT_QQ_EXIT_DELAY);

    if exit_with_qq {
        if qq_exit_delay > 0 {
            println!("[监控] QQ 退出监控已启用，缓冲时间 {} 秒（使用 --no-exit-with-qq 可禁用）", qq_exit_delay);
        } else {
            println!("[监控] QQ 退出监控已启用，QQ 退出后立即退出（使用 --no-exit-with-qq 可禁用）");
        }
        let _ = std::io::stdout().flush();
    }

    // 过滤掉 CLI 专用参数，不传递给 pmhq
    let pmhq_args: Vec<String> = args
        .iter()
        .filter(|a| {
            *a != "--exit-with-qq"
                && *a != "--no-exit-with-qq"
                && !a.starts_with("--qq-exit-delay=")
        })
        .cloned()
        .collect();

    // 检查 QQ 路径
    let detected_qq_path = qq::detect_qq_path(&exe_dir, &args);

    migrate::migrate_old_files(&exe_dir);

    let llbot_dir = exe_dir.join("bin/llbot");

    // 查找可用的 node（本地目录 -> 系统 PATH -> 自动下载）
    let node_path = match util::find_usable_node(&exe_dir) {
        Some(path) => path,
        None => {
            println!("未找到可用的 Node.js (需要版本 >= 24)");
            println!("正在自动下载 Node.js...");
            match updater::download_and_install_node(&exe_dir) {
                Ok(path) => path,
                Err(e) => {
                    eprintln!("下载 Node.js 失败: {}", e);
                    eprintln!("请手动安装 Node.js >= 24 或将其放置到 bin/llbot/ 目录");
                    util::wait_exit(1);
                }
            }
        }
    };

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(&node_path) {
            let mut perms = metadata.permissions();
            if perms.mode() & 0o111 == 0 {
                perms.set_mode(perms.mode() | 0o755);
                if let Err(e) = std::fs::set_permissions(&node_path, perms) {
                    eprintln!("警告: 设置 node 执行权限失败: {}", e);
                }
            }
        }
    }
    if !llbot_dir.join("llbot.js").exists() {
        eprintln!(
            "错误: 未找到 llbot.js: {}",
            llbot_dir.join("llbot.js").display()
        );
        util::wait_exit(1);
    }

    let port = util::find_available_port(DEFAULT_PORT, PORT_RANGE_END).unwrap_or_else(|| {
        eprintln!("错误: 无法找到可用端口 ({}-{})", DEFAULT_PORT, PORT_RANGE_END);
        util::wait_exit(1);
    });

    println!("LLBot CLI 启动器");
    println!("================");
    println!("端口: {}", port);
    println!();

    let mut cmd = Command::new(&pmhq_exe);
    cmd.arg("--port").arg(port.to_string());

    // 如果检测到 QQ 路径，传递给 pmhq
    if let Some(ref qq_path) = detected_qq_path {
        println!("检测到 QQ 路径: {}", qq_path);
        cmd.arg(format!("--qq-path={}", qq_path));
    }

    if !pmhq_args.is_empty() {
        cmd.args(&pmhq_args);
    }
    
    cmd.arg("--sub-cmd-workdir")
        .arg(&llbot_dir)
        .arg("--sub-cmd")
        .arg(&node_path)
        .arg("--enable-source-maps")
        .arg("llbot.js")
        .arg("--")
        .arg(format!("--pmhq-port={}", port));

    let mut child: GroupChild = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .group_spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("启动 pmhq 失败: {}", e);
            util::wait_exit(1);
        }
    };

    #[cfg(target_os = "windows")]
    if let Err(e) = windows_job::assign_to_job_object(&mut child) {
        eprintln!("警告: 无法设置进程保护: {}", e);
    }

    let child_arc: Arc<Mutex<Option<GroupChild>>> = Arc::new(Mutex::new(None));
    let child_for_handler = child_arc.clone();
    
    ctrlc::set_handler(move || {
        if let Ok(mut guard) = child_for_handler.lock() {
            if let Some(ref mut c) = *guard {
                let _ = c.kill();
            }
        }
        std::process::exit(0);
    })
    .ok();

    // 处理 SIGHUP（终端关闭）和 SIGTERM，确保子进程被清理
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGHUP, SIGTERM};
        use signal_hook::iterator::Signals;

        let child_for_signal = child_arc.clone();
        let mut signals = Signals::new(&[SIGHUP, SIGTERM]).expect("无法注册信号处理");
        thread::spawn(move || {
            for _sig in signals.forever() {
                if let Ok(mut guard) = child_for_signal.lock() {
                    if let Some(ref mut c) = *guard {
                        let _ = c.kill();
                    }
                }
                std::process::exit(0);
            }
        });
    }

    let stdout = child.inner().stdout.take();
    let stderr = child.inner().stderr.take();

    // 把 child 移入 Arc，供 ctrlc handler 使用
    *child_arc.lock().unwrap() = Some(child);
    let child_for_wait = child_arc.clone();

    if let Some(stdout) = stdout {
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    println!("{}", line);
                }
            }
        });
    }

    if let Some(stderr) = stderr {
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
    let show_terminal_qr = util::should_show_terminal_qrcode(&exe_dir, &pmhq_args);

    login::start_login_listener(port, logged_in.clone(), qrcode_path, show_terminal_qr);

    // 启动 QQ 进程监控（如果启用）
    if exit_with_qq {
        println!("[监控] 正在启动监控线程...");
        let child_for_qq_monitor = child_arc.clone();
        thread::spawn(move || {
            println!("[监控] 监控线程已启动");
            let client = pmhq_client::PMHQClient::new(port);

            // 等待获取 QQ PID
            println!("[监控] 等待 QQ 进程启动...");
            let mut qq_pid = loop {
                match client.get_qq_pid() {
                    Some(pid) => {
                        println!("[监控] 检测到 QQ 进程 (PID: {})", pid);
                        break pid;
                    }
                    None => {
                        // 静默重试，不输出日志避免刷屏
                    }
                }
                thread::sleep(Duration::from_secs(2));
            };

            println!("[监控] 开始监控 QQ 进程");

            loop {
                thread::sleep(Duration::from_secs(2));

                let running = qq::is_process_running(qq_pid);
                if !running {
                    println!("[监控] 检测到 QQ 进程 (PID: {}) 已退出", qq_pid);

                    // 如果缓冲时间为 0，立即退出
                    if qq_exit_delay == 0 {
                        println!("[监控] 正在退出所有进程...");
                        if let Ok(mut guard) = child_for_qq_monitor.lock() {
                            if let Some(ref mut c) = *guard {
                                let _ = c.kill();
                            }
                        }
                        std::process::exit(0);
                    }

                    println!("[监控] 将在 {} 秒后退出程序...", qq_exit_delay);

                    // 等待缓冲时间，期间检查是否恢复
                    for _ in 0..qq_exit_delay {
                        thread::sleep(Duration::from_secs(1));

                        // 检查是否恢复
                        if let Some(new_pid) = client.get_qq_pid() {
                            println!("[监控] QQ 进程已恢复 (新 PID: {})", new_pid);
                            qq_pid = new_pid;
                            break;
                        }
                    }

                    // 再次检查是否已恢复
                    if qq::is_process_running(qq_pid) {
                        continue;
                    }

                    // 最终检查
                    if let Some(new_pid) = client.get_qq_pid() {
                        println!("[监控] QQ 进程已恢复 (新 PID: {})", new_pid);
                        qq_pid = new_pid;
                        continue;
                    }

                    println!("[监控] 正在退出所有进程...");
                    if let Ok(mut guard) = child_for_qq_monitor.lock() {
                        if let Some(ref mut c) = *guard {
                            let _ = c.kill();
                        }
                    }
                    std::process::exit(0);
                }
            }
        });
    }

    // 等待子进程结束
    let pmhq_exit_status = loop {
        thread::sleep(Duration::from_millis(100));
        if let Ok(mut guard) = child_for_wait.lock() {
            if let Some(ref mut c) = *guard {
                match c.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            eprintln!("pmhq 退出，状态码: {:?}", status.code());
                        }
                        break status.code();
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("等待 pmhq 失败: {}", e);
                        break None;
                    }
                }
            } else {
                break None;
            }
        }
    };

    // pmhq 退出后，如果启用了 QQ 退出监控且有缓冲时间，等待后再退出
    // （pmhq 自身会在 QQ 退出时退出，此时 CLI 的监控线程来不及执行延迟逻辑）
    if exit_with_qq && qq_exit_delay > 0 {
        println!("[监控] pmhq 已退出，等待 {} 秒后退出...", qq_exit_delay);
        let _ = std::io::stdout().flush();

        for i in 0..qq_exit_delay {
            thread::sleep(Duration::from_secs(1));

            let remaining = qq_exit_delay - i - 1;
            if remaining > 0 && remaining % 5 == 0 {
                println!("[监控] 还有 {} 秒退出...", remaining);
            }
        }
    }

    std::process::exit(pmhq_exit_status.unwrap_or(1));
}

