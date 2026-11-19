//! Library errors mapped to process exit codes.

use v2ex_rs::Error;

pub fn exit_code(e: &Error) -> u8 {
    match e {
        Error::NotFound => 3,
        Error::RateLimited { .. } => 4,
        Error::Restricted { .. } => 5,
        Error::Network(_) => 6,
        Error::Config(_) => 2,
        Error::Status(_) | Error::Decode(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use v2ex_rs::Error;

    #[test]
    fn exit_codes() {
        assert_eq!(
            [
                exit_code(&Error::NotFound),
                exit_code(&Error::RateLimited { reset: None }),
                exit_code(&Error::Restricted { node: "x".into() }),
                exit_code(&Error::Network("x".into())),
                exit_code(&Error::Status(500)),
                exit_code(&Error::Config("x".into()))
            ],
            [3, 4, 5, 6, 1, 2]
        );
    }
}
