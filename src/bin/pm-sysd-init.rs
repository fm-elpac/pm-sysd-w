//! pm-sysd-init 命令行入口
#![deny(unsafe_code)]

use std::process::ExitCode;

use pm_bin::{cli_arg, init_env_logger, pm_init};
pm_init!();

use pm_sysd_w::cli::init;

fn main() -> Result<(), ExitCode> {
    init_env_logger();

    if let Some(a) = cli_arg(print_version) {
        if a.first().map(String::as_str) == Some("--reload") {
            init::main_reload(&a)
        } else {
            init::main(a, print_version)
        }
    } else {
        // pm-bin 会处理 --version 和 --版本
        Ok(())
    }
}
