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
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const DEFAULT_PORT: u16 = 13000;
const PORT_RANGE_END: u16 = 14000;


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
        let status = Command::new(&pmhq_exe).args(&args).status();
        std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
    }

    // --version 先输出 CLI 版本，再转发给 pmhq
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("llbot-cli {}", env!("CARGO_PKG_VERSION"));
        let status = Command::new(&pmhq_exe).args(&args).status();
        std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
    }


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

    if !args.is_empty() {
        cmd.args(&args);
    }
    
    cmd.arg("--exit-with-qq-delay")
        .arg("15")
        .arg("--sub-cmd-workdir")
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
    let show_terminal_qr = util::should_show_terminal_qrcode(&exe_dir, &args);

    login::start_login_listener(port, logged_in.clone(), qrcode_path, show_terminal_qr);

    // 等待子进程结束
    let exit_code = loop {
        thread::sleep(Duration::from_millis(100));
        if let Ok(mut guard) = child_for_wait.lock() {
            if let Some(ref mut c) = *guard {
                match c.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            eprintln!("pmhq 退出，状态码: {:?}", status.code());
                        }
                        break status.code().unwrap_or(1);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("等待 pmhq 失败: {}", e);
                        break 1;
                    }
                }
            } else {
                break 1;
            }
        }
    };

    std::process::exit(exit_code);
}

