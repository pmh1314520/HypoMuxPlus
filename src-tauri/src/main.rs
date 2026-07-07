// 阻止 Windows 发行版弹出额外控制台窗口，请勿删除！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 单二进制多入口：按命令行参数区分角色（服务主体 / 安装 / 卸载 / 正常 GUI）
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--service") {
        // 由 SCM 拉起，作为 SYSTEM 服务运行 TUN 承载体
        let _ = hypomuxplus_lib::service::run();
        return;
    }
    if args.iter().any(|a| a == "--install-service") {
        // 提权实例：注册并启动服务
        std::process::exit(if hypomuxplus_lib::service::install().is_ok() { 0 } else { 1 });
    }
    if args.iter().any(|a| a == "--uninstall-service") {
        // 提权实例：停止并删除服务
        std::process::exit(if hypomuxplus_lib::service::uninstall().is_ok() { 0 } else { 1 });
    }
    hypomuxplus_lib::run()
}
