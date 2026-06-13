use std::hash::{Hash, Hasher};

use chrono::Local;
use mutual::sip::SipHasher13;

pub type LogLevelType = (&'static str, u8);
pub const ERROR: LogLevelType = ("ERROR", 4);
pub const WARNING: LogLevelType = (" WARN", 3);
pub const INFO: LogLevelType = (" INFO", 2);
pub const DEBUG: LogLevelType = ("DEBUG", 1);
pub const TRACE: LogLevelType = ("TRACE", 0);

#[cfg(feature = "log-trace")]
const MAX_LOG_LEVEL: LogLevelType = TRACE;
#[cfg(all(debug_assertions, not(feature = "log-trace")))]
const MAX_LOG_LEVEL: LogLevelType = DEBUG;
#[cfg(all(not(debug_assertions), not(feature = "log-trace")))]
const MAX_LOG_LEVEL: LogLevelType = INFO;

pub fn log(file_name: &str, level: LogLevelType, message: &str) {
    // make sure level is allowed right now 
    if level.1 < MAX_LOG_LEVEL.1 { return }

    // get file name hash
    let mut hasher = SipHasher13::new_with_keys(0, 0);
    file_name.hash(&mut hasher);
    let hash = hasher.finish();

    // get RGB components of hash
    let red = (hash >> 16) & 0xFF;
    let green = (hash >> 8) & 0xFF;
    let blue = hash & 0xFF;

    // get date time
    let datetime = Local::now();
    let datetime = datetime.format("%m/%d/%Y %H:%M:%S");

    // encode file name and level
    let file_name = pad_and_ellipsis(file_name, 30, ' ');
    let level = level.0;

    #[cfg(target_arch = "wasm32")]
    {
        let str = format!("\x1b[38;2;{red};{green};{blue}m{datetime} [{file_name}] {level} | {message}");
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&str));
    }

    #[cfg(not(target_arch = "wasm32"))]
    println!("\x1b[38;2;{red};{green};{blue}m{datetime} [{file_name}] {level} | {message}");
}

fn pad_and_ellipsis(input: &str, max_len: usize, pad_char: char) -> String {
    let input_len = input.chars().count();

    if input_len <= max_len {
        let padding_needed = max_len - input_len;
        format!("{}{}", String::from(pad_char).repeat(padding_needed), input)
    } else {
        let ellipsis = "...";
        let truncated_len = input_len - max_len + 3;
        let truncated_string: String = input.chars().skip(truncated_len).collect();
        format!("{}{}", ellipsis, truncated_string)
    }
}
