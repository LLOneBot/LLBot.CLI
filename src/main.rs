//! LLBot CLI - 启动器
//! 
//! 目录结构:
//! llbot.exe (本程序)
//! bin/
//!   pmhq/
//!     pmhq.exe
//!     pmhq.dll
//!     pmhq_config.json
//!   llbot/
//!     node.exe
//!     llbot.js
//!     ...

use std::env;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

const DEFAULT_PORT: u16 = 13000;
const PORT_RANGE_END: u16 = 14000;

fn main() {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    // 路径
    let pmhq_exe = exe_dir.join("bin/pmhq/pmhq.exe");
    let llbot_node = exe_dir.join("bin/llbot/node.exe");
    let llbot_js = exe_dir.join("bin/llbot/llbot.js");

    // 检查文件是否存在
    if !pmhq_exe.exists() {
        eprintln!("错误: 未找到 pmhq.exe: {}", pmhq_exe.display());
        wait_exit(1);
    }
    if !llbot_node.exists() {
        eprintln!("错误: 未找到 node.exe: {}", llbot_node.display());
        wait_exit(1);
    }
    if !llbot_js.exists() {
        eprintln!("错误: 未找到 llbot.js: {}", llbot_js.display());
        wait_exit(1);
    }

    // 寻找可用端口
    let port = find_available_port(DEFAULT_PORT, PORT_RANGE_END)
        .unwrap_or_else(|| {
            eprintln!("错误: 无法找到可用端口 ({}-{})", DEFAULT_PORT, PORT_RANGE_END);
            wait_exit(1);
        });

    println!("LLBot CLI 启动器");
    println!("================");
    println!("pmhq: {}", pmhq_exe.display());
    println!("node: {}", llbot_node.display());
    println!("llbot.js: {}", llbot_js.display());
    println!("端口: {}", port);
    println!();

    // 构建命令: pmhq.exe --port <port> --sub-cmd node.exe --enable-source-maps llbot.js -- --pmhq-port=<port>
    let mut cmd = Command::new(&pmhq_exe);
    cmd.arg("--port")
        .arg(port.to_string())
        .arg("--sub-cmd")
        .arg(&llbot_node)
        .arg("--enable-source-maps")
        .arg(&llbot_js)
        .arg("--")
        .arg(format!("--pmhq-port={}", port));

    // 传递额外的命令行参数
    let args: Vec<String> = env::args().skip(1).collect();
    if !args.is_empty() {
        cmd.args(&args);
    }

    println!("执行: {} --port {} --sub-cmd {} --enable-source-maps {} -- --pmhq-port={}", 
        pmhq_exe.display(),
        port,
        llbot_node.display(), 
        llbot_js.display(),
        port
    );
    println!();

    // 执行并等待
    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                eprintln!("pmhq 退出，状态码: {:?}", status.code());
            }
        }
        Err(e) => {
            eprintln!("启动 pmhq 失败: {}", e);
            wait_exit(1);
        }
    }
}

/// 寻找可用端口
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
