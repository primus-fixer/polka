// Без окна консоли в выпускной сборке Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Приложение Claude запускает «Полку» с `--mcp`: это посредник коннектора, без окна.
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        std::process::exit(polka_app_lib::mcp_relay());
    }
    polka_app_lib::run()
}
