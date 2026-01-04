//! 二维码终端显示和文件保存

use base64::Engine;
use qrcode::QrCode;
use std::fs;
use std::path::Path;

/// 在终端显示二维码（紧凑模式，类似 segno 的 compact=True）
pub fn print_qrcode_terminal(url: &str) {
    let code = match QrCode::new(url.as_bytes()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("生成二维码失败: {}", e);
            return;
        }
    };

    let width = code.width();
    let colors = code.to_colors();

    // 清屏
    print!("\x1B[2J\x1B[H");
    println!();

    // 使用 Unicode 半块字符实现紧凑显示
    // ▀ (上半块), ▄ (下半块), █ (全块), 空格
    // 白色用 █，黑色用空格

    // 上边距（白色）
    print!("█");
    for _ in 0..width + 2 {
        print!("█");
    }
    println!("█");

    // 每两行合并为一行
    for y in (0..width).step_by(2) {
        print!("██"); // 左边距
        
        for x in 0..width {
            let top_idx = y * width + x;
            let bottom_idx = (y + 1) * width + x;
            
            let top_dark = colors.get(top_idx).map(|c| *c == qrcode::Color::Dark).unwrap_or(false);
            let bottom_dark = if y + 1 < width {
                colors.get(bottom_idx).map(|c| *c == qrcode::Color::Dark).unwrap_or(false)
            } else {
                false
            };

            // 白=亮, 黑=暗
            match (top_dark, bottom_dark) {
                (false, false) => print!("█"), // 上下都白
                (true, true) => print!(" "),   // 上下都黑
                (false, true) => print!("▀"),  // 上白下黑
                (true, false) => print!("▄"),  // 上黑下白
            }
        }
        
        println!("██"); // 右边距
    }

    // 下边距
    print!("█");
    for _ in 0..width + 2 {
        print!("█");
    }
    println!("█");
    println!();
}

pub fn save_qrcode_image(png_base64: &str, save_path: &Path) -> Result<(), String> {
    let base64_data = if let Some(pos) = png_base64.find("base64,") {
        &png_base64[pos + 7..]
    } else {
        png_base64
    };

    let image_data = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| format!("Base64 解码失败: {}", e))?;

    fs::write(save_path, &image_data).map_err(|e| format!("保存文件失败: {}", e))?;

    Ok(())
}
