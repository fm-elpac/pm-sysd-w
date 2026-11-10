//! pm-sysd-init 命令行入口
#![deny(unsafe_code)]

use std::process::ExitCode;

use pm_bin::{cli_arg, init_env_logger, pm_init};
pm_init!();

use pm_sysd_w::cli::init;

fn main() -> Result<(), ExitCode> {
    init_env_logger();

    if let Some(a) = cli_arg(print_version) {
        init::main(a)
    } else {
        // pm-bin 会处理 --version 和 --版本
        Ok(())
    }
}
