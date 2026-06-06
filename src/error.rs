//! The library error type.

/// Errors produced by the lifemaint library.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A recurrence spec (`every`/`on`) could not be parsed.
    #[error("cannot parse schedule: {0}")]
    Schedule(String),

    /// A data file is missing, unreadable, or malformed.
    #[error("{0}")]
    DataFile(String),

    /// A task references a vendor id that is not defined.
    #[error("task {task_id} references unknown vendor {vendor_id}")]
    UnknownVendor { task_id: String, vendor_id: String },

    /// An I/O failure reading or writing a data file.
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Convenience alias for library results.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn schedule_display() {
        assert_eq!(
            Error::Schedule("bad".to_string()).to_string(),
            "cannot parse schedule: bad"
        );
    }

    #[test]
    fn unknown_vendor_display_names_both_ids() {
        let e = Error::UnknownVendor {
            task_id: "clean-drains".to_string(),
            vendor_id: "roto".to_string(),
        };
        let s = e.to_string();
        assert!(s.contains("clean-drains"));
        assert!(s.contains("roto"));
    }

    #[test]
    fn io_wraps_source() {
        let e = Error::Io {
            path: "tasks.yaml".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "nope"),
        };
        assert!(e.to_string().contains("tasks.yaml"));
        assert!(std::error::Error::source(&e).is_some());
    }
}
