use std::io;

/// All error types for the photo-tiler pipeline.
#[derive(thiserror::Error, Debug)]
pub enum PhotoTilerError {
    #[error("Input error: {0}")]
    Input(String),
    #[error("Georeferencing error: {0}")]
    Georeference(String),
    #[error("Transform error: {0}")]
    Transform(String),
    #[error("Tiling error: {0}")]
    Tiling(String),
    #[error("Output error: {0}")]
    Output(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, PhotoTilerError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_strings() {
        let e = PhotoTilerError::Input("bad file".into());
        assert_eq!(e.to_string(), "Input error: bad file");

        let e = PhotoTilerError::Georeference("no CRS".into());
        assert_eq!(e.to_string(), "Georeferencing error: no CRS");

        let e = PhotoTilerError::Transform("overflow".into());
        assert_eq!(e.to_string(), "Transform error: overflow");

        let e = PhotoTilerError::Tiling("too deep".into());
        assert_eq!(e.to_string(), "Tiling error: too deep");

        let e = PhotoTilerError::Output("disk full".into());
        assert_eq!(e.to_string(), "Output error: disk full");

        let e = PhotoTilerError::Validation("schema mismatch".into());
        assert_eq!(e.to_string(), "Validation error: schema mismatch");
    }

    #[test]
    fn from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file missing");
        let e: PhotoTilerError = io_err.into();
        assert!(matches!(e, PhotoTilerError::Io(_)));
        assert!(e.to_string().contains("file missing"));
    }
}
