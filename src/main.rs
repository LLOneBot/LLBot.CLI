//! LLBot CLI - 启动器

#[cfg(target_os = "windows")]
mod llbot_ipc;
mod updater;
mod migrate;
mod qq;
mod util;
mod windows_job;

use command_group::{CommandGroup, GroupChild};
use std::env;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const DEFAULT_PORT: u16 = 13000;
const PORT_RANGE_END: u16 = 14000;

// 从参数里取 --qq=<uin> (快速登录号), 没有返回 None.
fn extract_qq_uin(args: &[String]) -> Option<String> {
    args.iter()
        .find_map(|a| a.strip_prefix("--qq=").map(|s| s.to_string()))
}

fn print_help() {
    println!("llbot-cli {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("LLBot 命令行启动器");
    println!();
    println!("用法:");
    println!("  llbot [选项]");
    println!();
    println!("模式:");
    println!("  默认 headless   不启动 PMHQ, 直接拉起 LLBot 直连 QQ 协议, 二维码由 LLBot 打印到终端");
    println!("  --pmhq          PMHQ 模式: 启动 PMHQ attach QQ, LLBot 经 PMHQ 通信");
    println!();
    println!("选项:");
    println!("  --pmhq, --no-headless   切换为 PMHQ 模式 (默认 headless 直连)");
    println!("  --qq=<number>           快速登录 QQ 号 (两种模式均生效)");
    println!("  --update                检查并执行组件更新 (CLI / PMHQ / LLBot)");
    println!("  --help, -h              显示此帮助");
    println!("  --version, -v           显示版本");
    println!();
    println!("PMHQ 模式额外参数 (透传给 PMHQ):");
    println!("  --qq-path=<path>        QQ 可执行文件路径");
    println!("  --qq-console            启用 QQ 控制台日志");
    println!("  --debug                 调试模式");
    println!("  --work-dir=<path>       工作目录");
    println!("  --no-exit-with-qq       禁用 QQ 退出时自动退出 (默认启用)");
    println!("  --qq-exit-delay=<秒>    QQ 退出后缓冲时间 (默认 15 秒)");
    println!();
    println!("示例:");
    println!("  llbot                   headless 直连 (默认)");
    println!("  llbot --qq=123456789    快速登录");
    println!("  llbot --pmhq            PMHQ 模式");
    println!("  llbot --update          检查更新");
}

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

    // --help / --version: 只依赖 CLI 自身, 不触发组件下载/不依赖 pmhq
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        std::process::exit(0);
    }
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("llbot-cli {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

    // 模式: 默认 headless (LLBot 直连 QQ 协议, 不启动 pmhq);
    //       --pmhq / --no-headless 切回 PMHQ 模式 (pmhq attach QQ, llbot 走 --pmhq-port).
    let headless = !args.iter().any(|a| a == "--pmhq" || a == "--no-headless");

    // 启动时自动补齐必要组件（缺失才下载; headless 不下 pmhq）
    if let Err(e) = updater::ensure_required_components(&exe_dir, headless) {
        eprintln!("自动安装组件失败: {}", e);
        util::wait_exit(1);
    }

    migrate::migrate_old_files(&exe_dir);

    let llbot_dir = exe_dir.join("bin/llbot");

    // Auth Token 两种模式均必须存在; 放在 migrate 之后, 旧 data 目录可能刚被迁入
    let auth_token = match util::read_auth_token(&exe_dir) {
        Some(token) => token,
        None => {
            eprintln!(
                "错误: 没有 Auth Token ({} 不存在或内容为空)",
                llbot_dir.join("data/auth_token.txt").display()
            );
            eprintln!("请到 https://auth.luckylillia.com 获取");
            util::wait_exit(1);
        }
    };

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

    // Windows: 生成命名管道名. LLBot 据此 listen, CLI 作为 client 轮询 get_login_state 拿 uin/nickname
    // (PMHQ 已不再提供 getSelfInfo). headless 直接设在 node 进程上; PMHQ 模式经 pmhq 透传给孙进程.
    #[cfg(target_os = "windows")]
    let ipc_pipe_name = llbot_ipc::generate_pipe_name();

    println!("LLBot CLI 启动器");
    println!("================");

    // 按模式构建启动命令. proc_label 用于日志/退出提示.
    let (mut cmd, proc_label): (Command, &str) = if headless {
        println!("模式: headless (LLBot 直连)");
        println!();

        let mut cmd = Command::new(&node_path);
        cmd.current_dir(&llbot_dir);
        cmd.env("NODE_SKIP_PLATFORM_CHECK", "1");
        #[cfg(target_os = "windows")]
        cmd.env("LL_IPC_PIPE", &ipc_pipe_name);
        cmd.arg("--enable-source-maps").arg("llbot.js");
        // 透传快速登录 QQ 号
        if let Some(uin) = extract_qq_uin(&args) {
            println!("快速登录 QQ: {}", uin);
            cmd.arg("--").arg(format!("--qq={}", uin));
        }
        (cmd, "LLBot")
    } else {
        let pmhq_exe = match util::find_pmhq_exe(&exe_dir) {
            Some(path) => path,
            None => {
                eprintln!("错误: 未找到 pmhq 可执行文件");
                eprintln!("请确保 bin/pmhq/ 目录下存在 pmhq 或 pmhq-<platform>-<arch> 文件");
                util::wait_exit(1);
            }
        };

        let detected_qq_path = qq::detect_qq_path(&exe_dir, &args);

        let port = util::find_available_port(DEFAULT_PORT, PORT_RANGE_END).unwrap_or_else(|| {
            eprintln!("错误: 无法找到可用端口 ({}-{})", DEFAULT_PORT, PORT_RANGE_END);
            util::wait_exit(1);
        });

        println!("模式: PMHQ");
        println!("端口: {}", port);
        println!();

        let mut cmd = Command::new(&pmhq_exe);
        cmd.env("NODE_SKIP_PLATFORM_CHECK", "1");
        // 告诉 LLBot 走 PMHQ 中继 (而非直连): 与下面的 --pmhq-port 成对出现.
        // 经 pmhq 透传给 node/llbot 孙进程 (同 NODE_SKIP_PLATFORM_CHECK 的透传方式).
        cmd.env("QQ_USE_PMHQ", "1");
        cmd.arg("--port").arg(port.to_string());
        cmd.arg(format!("--auth-token={}", auth_token));

        #[cfg(target_os = "windows")]
        cmd.env("LL_IPC_PIPE", &ipc_pipe_name);

        if let Some(ref qq_path) = detected_qq_path {
            println!("检测到 QQ 路径: {}", qq_path);
            cmd.arg(format!("--qq-path={}", qq_path));
        }

        // 透传用户参数 (去掉 CLI 自身的模式控制 flag)
        for a in args
            .iter()
            .filter(|a| a.as_str() != "--pmhq" && a.as_str() != "--no-headless")
        {
            cmd.arg(a);
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

        (cmd, "PMHQ")
    };

    let mut child: GroupChild = match cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .group_spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("启动 {} 失败: {}", proc_label, e);
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

    // 登录信息 (uin/昵称) 经 LLBot 命名管道获取并设置窗口标题; 二维码由 LLBot 自身打印到终端.
    #[cfg(target_os = "windows")]
    llbot_ipc::start_login_listener(ipc_pipe_name);

    // 等待子进程结束
    let exit_code = loop {
        thread::sleep(Duration::from_millis(100));
        if let Ok(mut guard) = child_for_wait.lock() {
            if let Some(ref mut c) = *guard {
                match c.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            eprintln!("{} 退出，状态码: {:?}", proc_label, status.code());
                        }
                        break status.code().unwrap_or(1);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("等待 {} 失败: {}", proc_label, e);
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
