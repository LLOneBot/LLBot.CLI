fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");  // 图标文件
        // 可选：设置版本信息
        // res.set("ProductName", "LLBot");
        // res.set("FileDescription", "LLBot CLI Launcher");
        // res.set("LegalCopyright", "Copyright © 2024");
        res.compile().unwrap();
    }
}
