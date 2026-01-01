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
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_PORT: u16 = 13000;
const PORT_RANGE_END: u16 = 14000;

fn main() {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));

    // 检查当前目录是否存在 data 目录，如果存在则移动到 bin/llbot/
    let data_dir = exe_dir.join("data");
    let target_data_dir = exe_dir.join("bin/llbot/data");
    if data_dir.exists() && data_dir.is_dir() {
        println!("检测到 data 目录，正在移动到 bin/llbot/...");
        // 如果目标已存在，先删除
        if target_data_dir.exists() {
            let _ = fs::remove_dir_all(&target_data_dir);
        }
        if let Err(_) = fs::rename(&data_dir, &target_data_dir) {
            // rename 跨分区可能失败，回退到复制+删除
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

    // 检查当前目录是否存在 pmhq_config.json，如果存在则移动到 bin/pmhq/
    let pmhq_config = exe_dir.join("pmhq_config.json");
    let target_pmhq_config = exe_dir.join("bin/pmhq/pmhq_config.json");
    if pmhq_config.exists() && pmhq_config.is_file() {
        println!("检测到 pmhq_config.json，正在移动到 bin/pmhq/...");
        // 如果目标已存在，先删除
        if target_pmhq_config.exists() {
            let _ = fs::remove_file(&target_pmhq_config);
        }
        if let Err(_) = fs::rename(&pmhq_config, &target_pmhq_config) {
            // rename 跨分区可能失败，回退到复制+删除
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

    // 路径
    let pmhq_exe = exe_dir.join("bin/pmhq/pmhq.exe");
    let llbot_dir = exe_dir.join("bin/llbot");

    // 检查文件是否存在
    if !pmhq_exe.exists() {
        eprintln!("错误: 未找到 pmhq.exe: {}", pmhq_exe.display());
        wait_exit(1);
    }
    if !llbot_dir.join("node.exe").exists() {
        eprintln!("错误: 未找到 node.exe: {}", llbot_dir.join("node.exe").display());
        wait_exit(1);
    }
    if !llbot_dir.join("llbot.js").exists() {
        eprintln!("错误: 未找到 llbot.js: {}", llbot_dir.join("llbot.js").display());
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
    println!("llbot目录: {}", exe_dir.join("bin/llbot").display());
    println!("端口: {}", port);
    println!();

    // 构建命令: pmhq.exe --port <port> --sub-cmd-workdir <llbot_dir> --sub-cmd node.exe --enable-source-maps llbot.js -- --pmhq-port=<port>
    let mut cmd = Command::new(&pmhq_exe);
    cmd.arg("--port")
        .arg(port.to_string())
        .arg("--sub-cmd-workdir")
        .arg(&llbot_dir)
        .arg("--sub-cmd")
        .arg("node.exe")
        .arg("--enable-source-maps")
        .arg("llbot.js")
        .arg("--")
        .arg(format!("--pmhq-port={}", port));

    // 传递额外的命令行参数
    let args: Vec<String> = env::args().skip(1).collect();
    if !args.is_empty() {
        cmd.args(&args);
    }

    println!("执行: {} --port {} --sub-cmd-workdir {} --sub-cmd node.exe --enable-source-maps llbot.js -- --pmhq-port={}", 
        pmhq_exe.display(),
        port,
        llbot_dir.display(),
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

/// 递归复制目录
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
