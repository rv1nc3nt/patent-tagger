// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;

fn main() {
    // SPEC 7.7: "the same binary accepts subcommands and then runs without
    // opening a window" - any argv beyond the binary name itself means CLI
    // mode; no arguments (a normal launch) falls through to the GUI.
    if std::env::args().len() > 1 {
        app_lib::attach_parent_console();
        let cli = app_lib::cli::Cli::parse();
        std::process::exit(app_lib::cli::run(cli));
    }
    app_lib::run();
}
