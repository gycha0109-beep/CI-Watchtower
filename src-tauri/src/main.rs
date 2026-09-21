#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ci_watchtower_lib::run();
}
