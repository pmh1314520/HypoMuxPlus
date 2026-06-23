// 阻止 Windows 发行版弹出额外控制台窗口，请勿删除！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hypomuxplus_lib::run()
}
