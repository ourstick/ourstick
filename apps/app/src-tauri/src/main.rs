//! The desktop app's entry point; the app itself is the library (ADR 0027).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ourstick_app_lib::run()
}
