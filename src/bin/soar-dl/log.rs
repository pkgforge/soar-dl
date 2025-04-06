use std::sync::atomic::{AtomicBool, Ordering};

static QUIET_MODE: AtomicBool = AtomicBool::new(false);

pub fn init(quiet: bool) {
    QUIET_MODE.store(quiet, Ordering::SeqCst);
}

pub fn is_quiet() -> bool {
    QUIET_MODE.load(Ordering::SeqCst)
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        if !$crate::log::is_quiet() {
            println!("{}", format!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        if !$crate::log::is_quiet() {
            eprintln!("{}", format!($($arg)*));
        }
    };
}
