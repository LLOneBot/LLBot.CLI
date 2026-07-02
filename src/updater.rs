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

use crate::util;

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
        "macos" => "macos",
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
pub fn kill_process(pid: u32) -> Result<(), String> {
    fn decode_taskkill_output(bytes: &[u8]) -> String {
        use std::ptr;
        use winapi::um::stringapiset::MultiByteToWideChar;
        use winapi::um::winnls::GetOEMCP;

        if bytes.is_empty() {
            return String::new();
        }

        unsafe {
            let code_page = GetOEMCP();
            let wide_len = MultiByteToWideChar(
                code_page,
                0,
                bytes.as_ptr() as *const i8,
                bytes.len() as i32,
                ptr::null_mut(),
                0,
            );

            if wide_len <= 0 {
                return String::from_utf8_lossy(bytes).to_string();
            }

            let mut wide = vec![0u16; wide_len as usize];
            let written = MultiByteToWideChar(
                code_page,
                0,
                bytes.as_ptr() as *const i8,
                bytes.len() as i32,
                wide.as_mut_ptr(),
                wide_len,
            );

            if written <= 0 {
                return String::from_utf8_lossy(bytes).to_string();
            }

            String::from_utf16_lossy(&wide)
        }
    }

    let output = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output()
        .map_err(|e| format!("启动 taskkill 失败: {}", e))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = decode_taskkill_output(&output.stdout);
    let stderr = decode_taskkill_output(&output.stderr);
    let message = if !stderr.trim().is_empty() {
        stderr.trim().to_string()
    } else if !stdout.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        format!("taskkill 失败，退出码: {:?}", output.status.code())
    };

    // 进程可能在我们调用前后就退出了；这种“找不到进程”视为成功。
    if message.contains("没有找到进程")
        || message.contains("未找到进程")
        || message.to_lowercase().contains("not found")
    {
        return Ok(());
    }

    Err(message)
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

/// 启动时自动补齐必要组件（pmhq / llbot）。
///
/// - 仅在检测到缺失时才会联网下载
/// - 不做交互式确认
/// - headless 模式不需要 pmhq (LLBot 直连 QQ 协议), 跳过其下载
pub fn ensure_required_components(exe_dir: &Path, headless: bool) -> Result<(), String> {
    let packages = ComponentPackages::for_current_platform();
    let mut changed = false;

    // pmhq (仅非 headless 需要)
    if !headless && util::find_pmhq_exe(exe_dir).is_none() {
        println!("检测到 PMHQ 未安装，正在自动下载...");
        let info = fetch_package_info(&packages.pmhq_package)?;
        let url = get_tarball_url(&packages.pmhq_package, &info.version);
        download_and_extract(&url, &exe_dir.join("bin/pmhq"))?;
        changed = true;
    }

    // llbot（node + llbot.js）
    let llbot_dir = exe_dir.join("bin/llbot");
    let node_exe = util::get_exe_name("node");
    let node_path = llbot_dir.join(&node_exe);
    let llbot_js_path = llbot_dir.join("llbot.js");
    if !node_path.exists() || !llbot_js_path.exists() {
        println!("检测到 LLBot 组件未安装，正在自动下载...");
        let info = fetch_package_info(&packages.llbot_package)?;
        let url = get_tarball_url(&packages.llbot_package, &info.version);
        download_and_extract(&url, &llbot_dir)?;
        changed = true;
    }

    if changed {
        println!("组件安装完成");
        println!();
    }

    Ok(())
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
        let mut running = check_running_processes();
        // 避免误杀正在执行更新的当前进程
        let current_pid = std::process::id();
        running.retain(|(name, pid)| {
            !(name.eq_ignore_ascii_case("llbot.exe") && *pid == current_pid)
        });

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
                    match kill_process(*pid) {
                        Ok(()) => println!(" 完成"),
                        Err(e) => {
                            println!(" 失败");
                            eprintln!("  原因: {}", e);
                            eprintln!("  提示: 可尝试以管理员身份运行，或手动在任务管理器结束该进程");
                        }
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
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::os::windows::process::CommandExt;
    use winapi::um::winbase::{CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_CONSOLE};
    
    let current_exe = env::current_exe()
        .map_err(|e| format!("获取当前exe路径失败: {}", e))?;
    let current_exe_name = current_exe.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("llbot.exe");
    
    let pid = std::process::id();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let temp_dir = env::temp_dir().join(format!("llbot-cli-update-{}-{}", pid, ts));
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("创建临时目录失败: {}", e))?;
    
    download_and_extract(tarball_url, &temp_dir)?;
    
    let new_exe = find_exe_in_dir(&temp_dir)
        .ok_or("下载的更新包中未找到可执行文件")?;
    
    let backup_exe = exe_dir.join(format!("{}.bak", current_exe_name));
    let batch_script = temp_dir.join("_update.bat");
    
    // 批处理：备份 -> 替换（带重试）-> 启动新版本 -> 清理
    let script = format!(
r#"@echo off
setlocal EnableExtensions

echo Updating LLBot CLI... Please wait.

set "CURRENT={current}"
set "BACKUP={backup}"
set "NEWEXE={new_exe}"
set "TEMPDIR={temp_dir}"

set /a MAX_RETRY=10

echo Backing up current executable...
set /a i=0
:try_backup
if exist "%BACKUP%" del /f /q "%BACKUP%" >nul 2>&1
move /y "%CURRENT%" "%BACKUP%" >nul 2>&1
if not errorlevel 1 goto backup_ok
set /a i+=1
if %i% GEQ %MAX_RETRY% (
  echo [ERROR] Failed to backup current executable.
  echo It may still be running or you may lack permission.
  goto fail
)
echo Waiting for file to be released... (%i%/%MAX_RETRY%)
timeout /t 1 /nobreak >nul
goto try_backup

:backup_ok

echo Installing new executable...
set /a i=0
:try_copy
copy /y "%NEWEXE%" "%CURRENT%" >nul 2>&1
if not errorlevel 1 goto copy_ok
set /a i+=1
if %i% GEQ %MAX_RETRY% (
  echo [ERROR] Failed to copy new executable. Restoring...
  move /y "%BACKUP%" "%CURRENT%" >nul 2>&1
  goto fail
)
echo Retry copy... (%i%/%MAX_RETRY%)
timeout /t 1 /nobreak >nul
goto try_copy

:copy_ok

echo Update finished.
echo Press any key to continue...
pause

start "" "%CURRENT%"
start /b "" cmd /c "timeout /t 3 /nobreak >nul & rmdir /s /q ""%TEMPDIR%"" 2>nul"
goto :eof

:fail
echo.
echo Update failed.
echo Tips:
echo   1) Run as Administrator
echo   2) Make sure llbot/pmhq/QQ are stopped
echo   3) Avoid protected install locations
echo.
echo Press any key to close this window...
pause
"#,
        backup = backup_exe.display(),
        current = current_exe.display(),
        new_exe = new_exe.display(),
        temp_dir = temp_dir.display(),
    );
    
    // cmd.exe 对仅 LF 换行的 .bat 解析不稳定，可能导致多行粘连成一行。
    // 这里强制写入 CRLF。注意不要写入 UTF-8 BOM，cmd.exe 可能无法正确识别首行指令。
    let script_crlf = script.replace("\n", "\r\n");

    fs::write(&batch_script, script_crlf.as_bytes())
        .map_err(|e| format!("创建更新脚本失败: {}", e))?;
    
    println!("启动更新脚本，程序即将退出...");

    // 说明：有些宿主（VS Code/终端）会把当前进程放进 JobObject，并在进程退出时
    // 连带杀掉子进程，导致更新脚本“看起来没有启动”。这里用 BREAKAWAY + NEW_CONSOLE
    // 尽量让更新脚本独立存活。
    let mut cmd = Command::new("cmd");
    // 使用 /C 执行更新脚本。按键等待放在脚本内部，避免外层命令拼接带来的解析差异。
    // 另外把工作目录切到临时目录，避免路径引号/转义问题。
    cmd.current_dir(&temp_dir)
        .arg("/C")
        .arg("call _update.bat");
    cmd.creation_flags(CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_CONSOLE);

    cmd.spawn()
        .map_err(|e| format!("启动更新脚本失败: {}（可能需要管理员权限，或被 JobObject 限制）", e))?;
    
    std::process::exit(0);
}

#[cfg(not(target_os = "windows"))]
fn self_update(tarball_url: &str, exe_dir: &Path) -> Result<(), String> {
    use std::env;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};
    
    let current_exe = env::current_exe()
        .map_err(|e| format!("获取当前exe路径失败: {}", e))?;
    let current_exe_name = current_exe.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("llbot");
    
    let pid = std::process::id();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let temp_dir = env::temp_dir().join(format!("llbot-cli-update-{}-{}", pid, ts));
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

/// 获取 llonebot-node 包名
fn get_node_package_name() -> String {
    let (os_name, arch_name) = get_platform_info();
    format!("llonebot-node-{}-{}", os_name, arch_name)
}

/// 下载并安装 Node.js（从 npm registry）
pub fn download_and_install_node(exe_dir: &Path) -> Result<std::path::PathBuf, String> {
    let package_name = get_node_package_name();
    println!("正在获取 {} 最新版本...", package_name);

    let info = fetch_package_info(&package_name)?;
    println!("将下载 {} v{}", package_name, info.version);

    let url = get_tarball_url(&package_name, &info.version);

    let llbot_dir = exe_dir.join("bin/llbot");
    fs::create_dir_all(&llbot_dir)
        .map_err(|e| format!("创建目录失败: {}", e))?;

    download_node_from_npm(&url, &llbot_dir)
}

fn download_node_from_npm(url: &str, llbot_dir: &Path) -> Result<std::path::PathBuf, String> {
    println!("下载中: {}", url);

    let resp = ureq::get(url)
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .call()
        .map_err(|e| format!("请求失败: {}", e))?;

    if resp.status() != 200 {
        return Err(format!("HTTP 错误: {}", resp.status()));
    }

    let content_length = resp.header("content-length")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    println!("下载中... ({}KB)", content_length / 1024);

    let mut data = Vec::with_capacity(content_length);
    resp.into_reader()
        .read_to_end(&mut data)
        .map_err(|e| format!("读取数据失败: {}", e))?;

    println!("下载完成，解压中...");

    // 解压 npm 包（tgz 格式）
    let gz = flate2::read::GzDecoder::new(BufReader::new(data.as_slice()));
    let mut archive = tar::Archive::new(gz);

    let node_exe = util::get_exe_name("node");
    let node_path = llbot_dir.join(&node_exe);

    // npm 包结构: package/node 或 package/node.exe
    let expected_name = format!("package/{}", node_exe);

    for entry in archive.entries().map_err(|e| format!("读取 tar 失败: {}", e))? {
        let mut entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
        let path = entry.path().map_err(|e| format!("获取路径失败: {}", e))?;

        if path.to_string_lossy() == expected_name {
            entry.unpack(&node_path).map_err(|e| format!("解压失败: {}", e))?;

            #[cfg(not(target_os = "windows"))]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&node_path, fs::Permissions::from_mode(0o755))
                    .map_err(|e| format!("设置权限失败: {}", e))?;
            }

            println!("Node.js 安装完成");
            return Ok(node_path);
        }
    }

    Err("npm 包中未找到 node 可执行文件".to_string())
}
