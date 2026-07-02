//! LLBot IPC 客户端 (仅 Windows): 通过命名管道向 LLBot 轮询登录状态, 登录后拿 uin / nickname.
//!
//! LLBot 是 server, CLI 是 client. 协议 JSON Lines (UTF-8, '\n' 分隔):
//!   CLI   -> LLBot: {"type":"request","id":"1","method":"get_login_state"}
//!   LLBot -> CLI  : {"type":"response","id":"1","data":{"state":"logged_in","uin":"...","nickname":"..."}}
//!
//! state 取值: initializing / need_qrcode / waiting_confirm / logged_in / expired / cancelled.
//! 二维码不在这里处理 -- LLBot 自身会打印到终端 (CLI 已转发其 stdout), 这里只等 logged_in 拿 uin/昵称.
//! 管道名由 CLI 生成并经 env LL_IPC_PIPE 透传给 LLBot (pmhq 的孙进程).

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::ffi::OsStrExt;
use std::process;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use winapi::um::wincon::SetConsoleTitleW;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
// 首次拿到 logged_in 后, 最多再等这么多次轮询补 nickname (uin 先到、nickname 后到), 拿不到就用 uin 收尾.
const NICK_GRACE_POLLS: u32 = 10;

/// 生成唯一管道名 (不含 \\.\pipe\ 前缀). 约定: luckylillia-llbot-{pid}-{unique}.
pub fn generate_pipe_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("luckylillia-llbot-{}-{:x}", process::id(), nanos)
}

/// 启动后台线程: 连接命名管道, 轮询 get_login_state, 登录成功后设置控制台标题 + 打印登录信息.
pub fn start_login_listener(pipe_name: String) {
    thread::spawn(move || run(&pipe_name));
}

fn run(pipe_name: &str) {
    let path = format!(r"\\.\pipe\{}", pipe_name);
    // LLBot 由 pmhq 作为孙进程拉起, 需要等它 listen; 连接失败 (管道未就绪 / 断开) 就退避重试.
    loop {
        if let Ok(pipe) = OpenOptions::new().read(true).write(true).open(&path) {
            if poll_connection(pipe) {
                return; // 登录完成, 收工
            }
            // 连接断开但还没登录 -> 重连
        }
        thread::sleep(RECONNECT_DELAY);
    }
}

struct LoginState {
    state: String,
    uin: String,
    nickname: String,
}

// 在一条已连接管道上轮询. 登录完成返回 true; 连接出错返回 false (触发重连).
fn poll_connection(pipe: File) -> bool {
    let mut writer = match pipe.try_clone() {
        Ok(w) => w,
        Err(_) => return false,
    };
    let mut reader = BufReader::new(pipe);
    let mut next_id: u64 = 0;

    let mut printed = false; // 成功横幅只打一次
    let mut title_nick: Option<String> = None; // 已写入标题的 nickname, 变化才重设
    let mut logged_polls: u32 = 0; // 首次 logged_in 之后的轮询计数 (给 nickname 留宽限)

    loop {
        next_id += 1;
        let id = next_id.to_string();
        let req = format!(
            "{{\"type\":\"request\",\"id\":\"{}\",\"method\":\"get_login_state\"}}\n",
            id
        );
        if writer.write_all(req.as_bytes()).is_err() || writer.flush().is_err() {
            return false;
        }

        match read_response(&mut reader, &id) {
            Ok(Some(info)) if info.state == "logged_in" && !info.uin.is_empty() => {
                // 标题: nickname 有变化 (含从空到非空) 就更新
                if title_nick.as_deref() != Some(info.nickname.as_str()) {
                    set_console_title(&info.uin, &info.nickname);
                    title_nick = Some(info.nickname.clone());
                }

                logged_polls += 1;
                let nick_ready = !info.nickname.is_empty();
                let grace_over = logged_polls >= NICK_GRACE_POLLS;

                if !printed && (nick_ready || grace_over) {
                    print_login_banner(&info.uin, &info.nickname);
                    printed = true;
                }
                if printed && (nick_ready || grace_over) {
                    return true;
                }
            }
            Ok(_) => {}             // 其它状态 / 错误响应 / 非匹配行: 继续等登录完成
            Err(_) => return false, // 连接断开, 重连
        }

        thread::sleep(POLL_INTERVAL);
    }
}

// 读取与 expected_id 匹配的响应. 单飞 (发一条收一条), 多扫几行兜底异常输入.
// 返回 Ok(None) 表示错误响应/无 data; Err 表示管道已关闭.
fn read_response(
    reader: &mut impl BufRead,
    expected_id: &str,
) -> std::io::Result<Option<LoginState>> {
    for _ in 0..8 {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "pipe closed",
            ));
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("response") {
            continue;
        }
        if v.get("id").and_then(|i| i.as_str()) != Some(expected_id) {
            continue;
        }
        let data = match v.get("data") {
            Some(d) if d.is_object() => d,
            _ => return Ok(None), // error 响应或缺 data
        };
        return Ok(Some(LoginState {
            state: data
                .get("state")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            uin: data
                .get("uin")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            nickname: data
                .get("nickname")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
        }));
    }
    Ok(None)
}

fn print_login_banner(uin: &str, nickname: &str) {
    println!();
    println!("================");
    println!("登录成功!");
    println!("QQ号: {}", uin);
    if !nickname.is_empty() {
        println!("昵称: {}", nickname);
    }
    println!("================");
    println!();
}

fn set_console_title(uin: &str, nickname: &str) {
    let title = if nickname.is_empty() {
        format!("LLBot - {}", uin)
    } else {
        format!("LLBot - {}({})", nickname, uin)
    };
    let wide: Vec<u16> = OsStr::new(&title)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetConsoleTitleW(wide.as_ptr());
    }
}
