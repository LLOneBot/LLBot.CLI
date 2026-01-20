//! LLBot CLI - 启动器

mod pmhq_client;
mod qrcode_display;
mod updater;

use command_group::{CommandGroup, GroupChild};
use pmhq_client::PMHQClient;
use qrcode_display::{print_qrcode_terminal, save_qrcode_image};
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const DEFAULT_PORT: u16 = 13000;
const PORT_RANGE_END: u16 = 14000;
const QQ_DOWNLOAD_URL: &str = "https://dldir1v6.qq.com/qqfile/qq/QQNT/c50d6326/QQ9.9.22.40768_x64.exe";

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

fn find_pmhq_exe(exe_dir: &Path) -> Option<PathBuf> {
    let pmhq_dir = exe_dir.join("bin/pmhq");
    
    let platform_arch = if cfg!(target_os = "windows") {
        "win-x64"
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "x86_64") {
            "linux-x64"
        } else if cfg!(target_arch = "aarch64") {
            "linux-arm64"
        } else {
            ""
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "macos-arm64"
        } else {
            "macos-x64"
        }
    } else {
        ""
    };
    
    if !platform_arch.is_empty() {
        let arch_specific = pmhq_dir.join(get_exe_name(&format!("pmhq-{}", platform_arch)));
        if arch_specific.exists() {
            return Some(arch_specific);
        }
    }
    
    let generic = pmhq_dir.join(get_exe_name("pmhq"));
    if generic.exists() {
        return Some(generic);
    }
    
    None
}

fn main() {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    let args: Vec<String> = env::args().skip(1).collect();
    let pmhq_exe = match find_pmhq_exe(&exe_dir) {
        Some(path) => path,
        None => {
            eprintln!("错误: 未找到 pmhq 可执行文件");
            eprintln!("请确保 bin/pmhq/ 目录下存在 pmhq 或 pmhq-<platform>-<arch> 文件");
            wait_exit(1);
        }
    };

    // --help 直接转发给 pmhq
    if args.iter().any(|a| a == "--help" || a == "-h") {
        let status = Command::new(&pmhq_exe).args(&args).status();
        std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
    }

    // --version 先输出 CLI 版本，再转发给 pmhq
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("llbot-cli {}", env!("CARGO_PKG_VERSION"));
        let status = Command::new(&pmhq_exe).args(&args).status();
        std::process::exit(status.map(|s| s.code().unwrap_or(0)).unwrap_or(1));
    }

    // --update 检查并执行更新
    if args.iter().any(|a| a == "--update") {
        updater::run_update(&exe_dir);
        wait_exit(0);
    }

    // 检查 QQ 路径
    if cfg!(target_os = "windows") {
        let qq_path_arg = args.iter()
            .find(|a| a.starts_with("--qq-path="))
            .map(|a| a.trim_start_matches("--qq-path=").to_string());
        
        let qq_path_arg_invalid = qq_path_arg.as_ref()
            .map(|p| !Path::new(p).exists())
            .unwrap_or(false);
        
        if qq_path_arg_invalid {
            eprintln!("错误: 指定的 QQ 路径不存在: {}", qq_path_arg.as_ref().unwrap());
        }
        
        let qq_path = if qq_path_arg_invalid { None } else { qq_path_arg.or_else(get_qq_path_from_registry) };
        
        if qq_path.is_none() || !qq_path.as_ref().map(|p| Path::new(p).exists()).unwrap_or(false) {
            println!("未找到 QQ，是否下载并安装？(y/n)");
            let mut input = String::new();
            if std::io::stdin().read_line(&mut input).is_ok() {
                if input.trim().eq_ignore_ascii_case("y") {
                    if !download_and_install_qq() {
                        eprintln!("QQ 下载安装失败");
                        wait_exit(1);
                    }
                    println!("QQ 安装完成，请重新运行程序");
                    wait_exit(0);
                } else {
                    eprintln!("错误: 未找到 QQ，请安装 QQ 或使用 --qq-path 参数指定路径");
                    wait_exit(1);
                }
            }
        }
    }

    migrate_old_files(&exe_dir);

    let llbot_dir = exe_dir.join("bin/llbot");
    let node_exe = get_exe_name("node");
    let node_path = llbot_dir.join(&node_exe);

    if !node_path.exists() {
        eprintln!(
            "错误: 未找到 {}: {}",
            node_exe,
            node_path.display()
        );
        wait_exit(1);
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(&node_path) {
            let mut perms = metadata.permissions();
            if perms.mode() & 0o111 == 0 {
                perms.set_mode(perms.mode() | 0o755);
                if let Err(e) = fs::set_permissions(&node_path, perms) {
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
            wait_exit(1);
        }
    };

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

    let stdout = child.inner().stdout.take();
    let stderr = child.inner().stderr.take();

    // 把 child 移入 Arc，供 ctrlc handler 使用
    *child_arc.lock().unwrap() = Some(child);
    let child_for_wait = child_arc.clone();

    if let Some(stdout) = stdout {
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stdout);
            let mut out = std::io::stdout().lock();
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = writeln!(out, "{}", line);
                    let _ = out.flush();
                }
            }
        });
    }

    if let Some(stderr) = stderr {
        thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stderr);
            let mut err = std::io::stderr().lock();
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = writeln!(err, "{}", line);
                    let _ = err.flush();
                }
            }
        });
    }

    let logged_in = Arc::new(AtomicBool::new(false));
    let qrcode_path = exe_dir.join("qrcode.png");
    let show_terminal_qr = should_show_terminal_qrcode(&exe_dir, &args);

    start_login_listener(port, logged_in.clone(), qrcode_path, show_terminal_qr);

    // 等待子进程结束
    loop {
        thread::sleep(Duration::from_millis(100));
        if let Ok(mut guard) = child_for_wait.lock() {
            if let Some(ref mut c) = *guard {
                match c.try_wait() {
                    Ok(Some(status)) => {
                        if !status.success() {
                            eprintln!("pmhq 退出，状态码: {:?}", status.code());
                        }
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("等待 pmhq 失败: {}", e);
                        break;
                    }
                }
            } else {
                break;
            }
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
    println!("\n按任意键退出...");
    let _ = std::io::stdin().read_line(&mut String::new());
    std::process::exit(code);
}

#[cfg(target_os = "windows")]
fn get_qq_path_from_registry() -> Option<String> {
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

#[cfg(not(target_os = "windows"))]
fn get_qq_path_from_registry() -> Option<String> {
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
            match Command::new(&temp_file).arg("/S").status() {
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
