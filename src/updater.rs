//! 更新检查和下载模块

use serde::Deserialize;
use std::env::consts::{ARCH, OS};
use std::fs::{self, File};
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const NPM_OFFICIAL_REGISTRY: &str = "https://registry.npmjs.org";
const NPM_REGISTRY_MIRRORS: &[&str] = &[
    "https://registry.npmmirror.com",
    "https://mirrors.huaweicloud.com/repository/npm",
    "https://mirrors.cloud.tencent.com/npm",
];

const UPDATE_TIMEOUT_SECS: u64 = 15;
const DOWNLOAD_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Deserialize)]
struct NpmPackageInfo {
    version: String,
}

#[derive(Debug)]
pub struct UpdateInfo {
    pub name: String,
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub tarball_url: Option<String>,
}

#[derive(Debug)]
pub struct ComponentPackages {
    pub cli_package: String,
    pub pmhq_package: String,
    pub llbot_package: String,
}

impl ComponentPackages {
    pub fn for_current_platform() -> Self {
        let (os_name, arch_name) = get_platform_info();
        
        Self {
            cli_package: format!("llbot-cli-{}-{}", os_name, arch_name),
            pmhq_package: format!("pmhq-dist-{}-{}", os_name, arch_name),
            llbot_package: "llonebot-dist".to_string(),
        }
    }
}

fn get_platform_info() -> (&'static str, &'static str) {
    let os_name = match OS {
        "windows" => "win",
        "linux" => "linux",
        "macos" => "darwin",
        _ => OS,
    };
    
    let arch_name = match ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => ARCH,
    };
    
    (os_name, arch_name)
}

fn fetch_package_info(package_name: &str) -> Result<NpmPackageInfo, String> {
    let encoded_name = package_name.replace("/", "%2F");
    
    // 先尝试官方源
    let url = format!("{}/{}/latest", NPM_OFFICIAL_REGISTRY, encoded_name);
    match ureq::get(&url)
        .timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
        .call()
    {
        Ok(resp) if resp.status() == 200 => {
            if let Ok(info) = resp.into_json::<NpmPackageInfo>() {
                return Ok(info);
            }
        }
        _ => {}
    }
    
    // 官方源失败，并发尝试镜像源
    let (tx, rx) = mpsc::channel();
    
    for mirror in NPM_REGISTRY_MIRRORS {
        let tx = tx.clone();
        let url = format!("{}/{}/latest", mirror, encoded_name);
        thread::spawn(move || {
            if let Ok(resp) = ureq::get(&url)
                .timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
                .call()
            {
                if resp.status() == 200 {
                    if let Ok(info) = resp.into_json::<NpmPackageInfo>() {
                        let _ = tx.send(Some(info));
                        return;
                    }
                }
            }
            let _ = tx.send(None);
        });
    }
    
    drop(tx);
    
    for result in rx {
        if let Some(info) = result {
            return Ok(info);
        }
    }
    
    Err(format!("无法获取 {} 的包信息", package_name))
}

fn check_version_exists(package_name: &str, version: &str, registry: &str) -> bool {
    let encoded_name = package_name.replace("/", "%2F");
    let url = format!("{}/{}/{}", registry, encoded_name, version);
    
    ureq::get(&url)
        .timeout(Duration::from_secs(UPDATE_TIMEOUT_SECS))
        .call()
        .map(|r| r.status() == 200)
        .unwrap_or(false)
}

fn get_best_download_registry(package_name: &str, version: &str) -> String {
    let (tx, rx) = mpsc::channel();
    
    for mirror in NPM_REGISTRY_MIRRORS {
        let tx = tx.clone();
        let mirror = mirror.to_string();
        let pkg = package_name.to_string();
        let ver = version.to_string();
        
        thread::spawn(move || {
            if check_version_exists(&pkg, &ver, &mirror) {
                let _ = tx.send(Some(mirror));
            } else {
                let _ = tx.send(None);
            }
        });
    }
    
    drop(tx);
    
    for result in rx {
        if let Some(registry) = result {
            return registry;
        }
    }
    
    NPM_OFFICIAL_REGISTRY.to_string()
}

fn get_tarball_url(package_name: &str, version: &str) -> String {
    let best_registry = get_best_download_registry(package_name, version);
    let pkg_short_name = package_name.split('/').last().unwrap_or(package_name);
    format!("{}/{}/-/{}-{}.tgz", best_registry, package_name, pkg_short_name, version)
}

fn compare_versions(current: &str, latest: &str) -> bool {
    let parse_version = |v: &str| -> Vec<u32> {
        v.trim_start_matches('v')
            .trim_start_matches('V')
            .split('.')
            .filter_map(|s| s.split('-').next()?.parse().ok())
            .collect()
    };
    
    let current_parts = parse_version(current);
    let latest_parts = parse_version(latest);
    
    for i in 0..std::cmp::max(current_parts.len(), latest_parts.len()) {
        let c = current_parts.get(i).copied().unwrap_or(0);
        let l = latest_parts.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        } else if l < c {
            return false;
        }
    }
    false
}

pub fn check_update(name: &str, package_name: &str, current_version: &str) -> UpdateInfo {
    match fetch_package_info(package_name) {
        Ok(info) => {
            let has_update = compare_versions(current_version, &info.version);
            let tarball_url = if has_update {
                Some(get_tarball_url(package_name, &info.version))
            } else {
                None
            };
            UpdateInfo {
                name: name.to_string(),
                current_version: current_version.to_string(),
                latest_version: info.version,
                has_update,
                tarball_url,
            }
        }
        Err(e) => {
            eprintln!("检查 {} 更新失败: {}", name, e);
            UpdateInfo {
                name: name.to_string(),
                current_version: current_version.to_string(),
                latest_version: "未知".to_string(),
                has_update: false,
                tarball_url: None,
            }
        }
    }
}


pub fn get_local_version(exe_dir: &Path, component: &str) -> String {
    let package_json_path = match component {
        "pmhq" => exe_dir.join("bin/pmhq/package.json"),
        "llbot" | "node" => exe_dir.join("bin/llbot/package.json"),
        _ => return "未知".to_string(),
    };
    
    if let Ok(content) = fs::read_to_string(&package_json_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(version) = json.get("version").and_then(|v| v.as_str()) {
                return version.to_string();
            }
        }
    }
    "未安装".to_string()
}

#[cfg(target_os = "windows")]
pub fn check_running_processes() -> Vec<(String, u32)> {
    let mut running = Vec::new();
    
    let targets = ["llbot.exe", "pmhq.exe", "QQ.exe"];
    
    let output = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output();
    
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let name = parts[0].trim_matches('"');
                let pid_str = parts[1].trim_matches('"');
                
                for target in &targets {
                    if name.eq_ignore_ascii_case(target) {
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            running.push((name.to_string(), pid));
                        }
                    }
                }
            }
        }
    }
    running
}

#[cfg(not(target_os = "windows"))]
pub fn check_running_processes() -> Vec<(String, u32)> {
    Vec::new()
}

#[cfg(target_os = "windows")]
pub fn kill_process(pid: u32) -> bool {
    Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub fn kill_process(pid: u32) -> bool {
    Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn download_and_extract(tarball_url: &str, extract_dir: &Path) -> Result<(), String> {
    println!("下载中: {}", tarball_url);
    
    let resp = ureq::get(tarball_url)
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .call()
        .map_err(|e| format!("下载失败: {}", e))?;
    
    if resp.status() != 200 {
        return Err(format!("HTTP 错误: {}", resp.status()));
    }
    
    let content_length = resp.header("content-length")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    
    let mut data = Vec::with_capacity(content_length);
    resp.into_reader()
        .read_to_end(&mut data)
        .map_err(|e| format!("读取数据失败: {}", e))?;
    
    println!("下载完成，大小: {} KB", data.len() / 1024);
    
    fs::create_dir_all(extract_dir)
        .map_err(|e| format!("创建目录失败: {}", e))?;
    
    let temp_file = extract_dir.join("_temp_download.tgz");
    fs::write(&temp_file, &data)
        .map_err(|e| format!("保存临时文件失败: {}", e))?;
    
    println!("解压中...");
    
    let file = File::open(&temp_file)
        .map_err(|e| format!("打开临时文件失败: {}", e))?;
    let gz = flate2::read::GzDecoder::new(BufReader::new(file));
    let mut archive = tar::Archive::new(gz);
    
    let temp_extract = extract_dir.join("_temp_extract");
    fs::create_dir_all(&temp_extract)
        .map_err(|e| format!("创建临时解压目录失败: {}", e))?;
    
    archive.unpack(&temp_extract)
        .map_err(|e| format!("解压失败: {}", e))?;
    
    let package_dir = temp_extract.join("package");
    if package_dir.exists() {
        for entry in fs::read_dir(&package_dir).map_err(|e| format!("读取目录失败: {}", e))? {
            let entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
            let src = entry.path();
            let dst = extract_dir.join(entry.file_name());
            
            if dst.exists() {
                if dst.is_dir() {
                    let _ = fs::remove_dir_all(&dst);
                } else {
                    let _ = fs::remove_file(&dst);
                }
            }
            
            fs::rename(&src, &dst)
                .or_else(|_| copy_recursive(&src, &dst))
                .map_err(|e| format!("移动文件失败: {}", e))?;
        }
    }
    
    let _ = fs::remove_dir_all(&temp_extract);
    let _ = fs::remove_file(&temp_file);
    
    println!("解压完成");
    Ok(())
}

fn copy_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        fs::copy(src, dst)?;
    }
    Ok(())
}

pub fn prompt_yes_no(prompt: &str) -> bool {
    print!("{} [y/N]: ", prompt);
    io::stdout().flush().ok();
    
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        let input = input.trim().to_lowercase();
        return input == "y" || input == "yes";
    }
    false
}

pub fn run_update(exe_dir: &Path) {
    println!("LLBot 更新检查");
    println!("===============");
    println!();
    
    let packages = ComponentPackages::for_current_platform();
    let cli_version = env!("CARGO_PKG_VERSION");
    let pmhq_version = get_local_version(exe_dir, "pmhq");
    let llbot_version = get_local_version(exe_dir, "llbot");
    
    println!("检查更新中...");
    println!();
    
    let cli_update = check_update("LLBot CLI", &packages.cli_package, cli_version);
    let pmhq_update = check_update("PMHQ", &packages.pmhq_package, &pmhq_version);
    let llbot_update = check_update("LLBot", &packages.llbot_package, &llbot_version);
    
    println!("组件          当前版本        最新版本        状态");
    println!("----          --------        --------        ----");
    print_update_row(&cli_update);
    print_update_row(&pmhq_update);
    print_update_row(&llbot_update);
    println!();
    
    let updates: Vec<&UpdateInfo> = [&cli_update, &pmhq_update, &llbot_update]
        .into_iter()
        .filter(|u| u.has_update && u.tarball_url.is_some())
        .collect();
    
    if updates.is_empty() {
        println!("所有组件都是最新版本");
        return;
    }
    
    println!("发现 {} 个可用更新", updates.len());
    
    #[cfg(target_os = "windows")]
    {
        let running = check_running_processes();
        if !running.is_empty() {
            println!();
            println!("检测到以下进程正在运行:");
            for (name, pid) in &running {
                println!("  - {} (PID: {})", name, pid);
            }
            println!();
            
            if prompt_yes_no("是否关闭这些进程?") {
                for (name, pid) in &running {
                    print!("正在关闭 {}...", name);
                    if kill_process(*pid) {
                        println!(" 完成");
                    } else {
                        println!(" 失败");
                    }
                }
                println!();
            }
        }
    }
    
    if !prompt_yes_no("是否开始更新?") {
        println!("更新已取消");
        return;
    }
    
    println!();
    
    let mut need_self_update = false;
    
    for update in &updates {
        if update.name == "LLBot CLI" {
            need_self_update = true;
            continue;
        }
        
        let target_dir = match update.name.as_str() {
            "PMHQ" => exe_dir.join("bin/pmhq"),
            "LLBot" => exe_dir.join("bin/llbot"),
            _ => continue,
        };
        
        println!("更新 {}...", update.name);
        
        if let Some(ref url) = update.tarball_url {
            match download_and_extract(url, &target_dir) {
                Ok(()) => println!("{} 更新成功!", update.name),
                Err(e) => eprintln!("{} 更新失败: {}", update.name, e),
            }
        }
        println!();
    }
    
    if need_self_update {
        if let Some(cli_update) = updates.iter().find(|u| u.name == "LLBot CLI") {
            println!("更新 LLBot CLI...");
            if let Some(ref url) = cli_update.tarball_url {
                match self_update(url, exe_dir) {
                    Ok(()) => return,
                    Err(e) => eprintln!("LLBot CLI 更新失败: {}", e),
                }
            }
        }
    }
    
    println!("更新完成!");
}

fn print_update_row(info: &UpdateInfo) {
    let status = if info.has_update { "有更新" } else { "最新" };
    println!(
        "{:<12}  {:<14}  {:<14}  {}",
        info.name, info.current_version, info.latest_version, status
    );
}


#[cfg(target_os = "windows")]
fn self_update(tarball_url: &str, exe_dir: &Path) -> Result<(), String> {
    use std::env;
    
    let current_exe = env::current_exe()
        .map_err(|e| format!("获取当前exe路径失败: {}", e))?;
    let current_exe_name = current_exe.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("llbot.exe");
    
    let temp_dir = exe_dir.join("_cli_update_temp");
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;
    
    download_and_extract(tarball_url, &temp_dir)?;
    
    let new_exe = find_exe_in_dir(&temp_dir)
        .ok_or("下载的更新包中未找到可执行文件")?;
    
    let backup_exe = exe_dir.join(format!("{}.bak", current_exe_name));
    let batch_script = temp_dir.join("_update.bat");
    
    // 批处理：等待当前进程退出 -> 备份 -> 替换 -> 启动新版本 -> 清理
    let script = format!(
r#"@echo off
chcp 65001 >nul
echo 正在更新 LLBot CLI，请稍候...

:wait
timeout /t 1 /nobreak >nul
tasklist /FI "PID eq {pid}" 2>NUL | find /I "{pid}" >NUL
if not errorlevel 1 goto wait

echo 备份旧版本...
if exist "{backup}" del /f /q "{backup}"
move /y "{current}" "{backup}"

echo 安装新版本...
copy /y "{new_exe}" "{current}"

if errorlevel 1 (
    echo 更新失败，正在恢复...
    move /y "{backup}" "{current}"
    pause
    exit /b 1
)

echo 更新完成！
timeout /t 2 /nobreak >nul

start "" "{current}"
start /b "" cmd /c "timeout /t 3 /nobreak >nul & rmdir /s /q "{temp_dir}" 2>nul"
exit
"#,
        pid = std::process::id(),
        backup = backup_exe.display(),
        current = current_exe.display(),
        new_exe = new_exe.display(),
        temp_dir = temp_dir.display(),
    );
    
    fs::write(&batch_script, &script)
        .map_err(|e| format!("创建更新脚本失败: {}", e))?;
    
    println!("启动更新脚本，程序即将退出...");
    
    Command::new("cmd")
        .args(["/C", "start", "", "/MIN", batch_script.to_str().unwrap()])
        .spawn()
        .map_err(|e| format!("启动更新脚本失败: {}", e))?;
    
    std::process::exit(0);
}

#[cfg(not(target_os = "windows"))]
fn self_update(tarball_url: &str, exe_dir: &Path) -> Result<(), String> {
    use std::env;
    use std::os::unix::fs::PermissionsExt;
    
    let current_exe = env::current_exe()
        .map_err(|e| format!("获取当前exe路径失败: {}", e))?;
    let current_exe_name = current_exe.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("llbot");
    
    let temp_dir = exe_dir.join("_cli_update_temp");
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;
    
    download_and_extract(tarball_url, &temp_dir)?;
    
    let new_exe = find_exe_in_dir(&temp_dir)
        .ok_or("下载的更新包中未找到可执行文件")?;
    
    let backup_exe = exe_dir.join(format!("{}.bak", current_exe_name));
    
    if backup_exe.exists() {
        fs::remove_file(&backup_exe).ok();
    }
    fs::rename(&current_exe, &backup_exe)
        .map_err(|e| format!("备份失败: {}", e))?;
    
    fs::copy(&new_exe, &current_exe)
        .map_err(|e| {
            fs::rename(&backup_exe, &current_exe).ok();
            format!("复制新版本失败: {}", e)
        })?;
    
    fs::set_permissions(&current_exe, fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("设置权限失败: {}", e))?;
    
    let _ = fs::remove_dir_all(&temp_dir);
    
    println!("更新完成！请重新启动程序。");
    Ok(())
}

fn find_exe_in_dir(dir: &Path) -> Option<std::path::PathBuf> {
    let exe_name = if cfg!(target_os = "windows") { "llbot.exe" } else { "llbot" };
    
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == exe_name || name.starts_with("llbot") {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}
