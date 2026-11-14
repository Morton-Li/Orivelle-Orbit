// Release builds are GUI applications: they must not create or attach a console.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    orbit_desktop::run()
}
