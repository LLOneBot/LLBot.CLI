use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

pub fn should_show_terminal_qrcode(exe_dir: &Path, args: &[String]) -> bool {
    if cfg!(not(target_os = "windows")) {
        return true;
    }

    if args.iter().any(|a| a == "--headless") {
        return true;
    }

    let config_path = exe_dir.join("bin/pmhq/pmhq_config.json");
    if let Ok(content) = fs::read_to_string(&config_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            return json
                .get("headless")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        }
    }
    false
}

pub fn get_exe_name(base: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{}.exe", base)
    } else {
        base.to_string()
    }
}

pub fn find_pmhq_exe(exe_dir: &Path) -> Option<PathBuf> {
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

pub fn find_available_port(start: u16, end: u16) -> Option<u16> {
    for port in start..end {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Some(port);
        }
    }
    None
}

pub fn wait_exit(code: i32) -> ! {
    println!("\n按任意键退出...");
    let _ = std::io::stdin().read_line(&mut String::new());
    std::process::exit(code);
}
