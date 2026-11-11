#![cfg_attr(feature = "axstd", no_std)]
#![cfg_attr(feature = "axstd", no_main)]

#[cfg(feature = "axstd")]
use axstd::println;

/// ANSI CSI 前缀：ESC '[' + 属性参数 + 终止符 'm'
/// 1 = 加粗，32 = 绿色前景；0 = 重置全部属性
const GREEN: &str = "\x1b[1;32m";
const RESET: &str = "\x1b[0m";

#[cfg_attr(feature = "axstd", no_mangle)]
fn main() {
    println!("{GREEN}[WithColor]{RESET}: Hello, Arceos!");
}
