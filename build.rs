fn main() {
    #[cfg(target_os = "windows")]
    {
        // 静态链接 VC 运行时
        static_vcruntime::metabuild();
        
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.compile().unwrap();
    }
}
