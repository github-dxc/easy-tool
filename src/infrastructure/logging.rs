//! Logging setup shared by normal startup and file-preview startup.

use flexi_logger::{Cleanup, Criterion, Duplicate, FileSpec, Logger, Naming};

/// Initializes rotating file logs and mirrors info-level logs to stdout.
pub fn init_logging() -> Result<(), String> {
    Logger::try_with_str("info")
        .map_err(|err| format!("create logger failed: {err}"))?
        .log_to_file(FileSpec::default().directory("logs").basename("easy-tool"))
        .duplicate_to_stdout(Duplicate::Info)
        .rotate(
            Criterion::Size(10_000_000),
            Naming::Numbers,
            Cleanup::KeepLogFiles(7),
        )
        .start()
        .map_err(|err| format!("start logger failed: {err}"))?;

    Ok(())
}
