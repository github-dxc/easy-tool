pub mod app;
pub mod assets;
pub mod config;
pub mod features;
pub mod infrastructure;
pub mod platform;

slint::include_modules!();

pub use app::run;
pub use features::time_trans::window::init_time_trans_window;
pub use infrastructure::tray::init_tray_icon;
