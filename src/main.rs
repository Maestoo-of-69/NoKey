// No console window: NoKey is a GUI/tray app.
#![windows_subsystem = "windows"]

fn main() {
    std::process::exit(nokey::app::run());
}
