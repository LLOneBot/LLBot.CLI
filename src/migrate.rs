use std::fs;
use std::path::Path;

pub fn migrate_old_files(exe_dir: &Path) {
    // 迁移 data 目录
    let data_dir = exe_dir.join("data");
    let target_data_dir = exe_dir.join("bin/llbot/data");
    if data_dir.exists() && data_dir.is_dir() {
        println!("检测到 data 目录，正在移动到 bin/llbot/...");
        if target_data_dir.exists() {
            let _ = fs::remove_dir_all(&target_data_dir);
        }
        if fs::rename(&data_dir, &target_data_dir).is_err() {
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

    // 迁移 pmhq_config.json
    let pmhq_config = exe_dir.join("pmhq_config.json");
    let target_pmhq_config = exe_dir.join("bin/pmhq/pmhq_config.json");
    if pmhq_config.exists() && pmhq_config.is_file() {
        println!("检测到 pmhq_config.json，正在移动到 bin/pmhq/...");
        if target_pmhq_config.exists() {
            let _ = fs::remove_file(&target_pmhq_config);
        }
        if fs::rename(&pmhq_config, &target_pmhq_config).is_err() {
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
}

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
