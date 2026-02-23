use crate::pmhq_client::PMHQClient;
use crate::qrcode_display::{print_qrcode_terminal, save_qrcode_image};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub fn start_login_listener(
    port: u16,
    logged_in: Arc<AtomicBool>,
    qrcode_path: PathBuf,
    show_terminal_qr: bool,
) {
    thread::spawn(move || {
        let client = PMHQClient::new(port).with_timeout(Duration::from_secs(10));

        #[cfg(target_os = "windows")]
        start_windows_selfinfo_title_thread(client.clone());

        thread::sleep(Duration::from_secs(3));

        let logged_in_refresh = logged_in.clone();
        let client_refresh = client.clone();
        thread::spawn(move || loop {
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

#[cfg(target_os = "windows")]
fn start_windows_selfinfo_title_thread(client: PMHQClient) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    use winapi::um::wincon::SetConsoleTitleW;

    thread::spawn(move || loop {
        match client.get_self_info() {
            Ok(info) => {
                if !info.uin.is_empty() && !info.nickname.is_empty() {
                    let title = format!("LLBot - {}({})", info.nickname, info.uin);
                    let _ = set_windows_console_title(&title);
                    break;
                }
            }
            Err(_) => {
                // 忽略未就绪/临时错误，继续重试
            }
        }

        thread::sleep(Duration::from_secs(1));
    });

    fn set_windows_console_title(title: &str) -> Result<(), String> {
        let wide: Vec<u16> = OsStr::new(title)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let ok = unsafe { SetConsoleTitleW(wide.as_ptr()) };
        if ok == 0 {
            Err("SetConsoleTitleW 调用失败".to_string())
        } else {
            Ok(())
        }
    }
}
