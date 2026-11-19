//! Library errors mapped to process exit codes and user-facing messages.

use chrono::FixedOffset;
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

/// Chinese message for `e`; a rate-limit reset time is shown in `tz`.
pub fn message(e: &Error, tz: &FixedOffset) -> String {
    match e {
        Error::NotFound => "找不到这个帖子或节点".to_string(),
        Error::RateLimited { reset: Some(t) } => {
            format!(
                "接口额度已用完，{} 恢复",
                t.with_timezone(tz).format("%H:%M")
            )
        }
        Error::RateLimited { reset: None } => "接口额度已用完，稍后再试".to_string(),
        Error::Restricted { node } => format!("节点 {node} 需要登录才能查看"),
        Error::Network(s) => format!("网络错误：{s}"),
        Error::Status(c) => format!("站点返回 HTTP {c}"),
        Error::Decode(s) => format!("无法解析站点的响应：{s}"),
        Error::Config(s) => format!("设置有误：{s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone, Utc};
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

    #[test]
    fn rate_limited_message_uses_local_time() {
        let tz = FixedOffset::east_opt(8 * 3600).unwrap();
        let e = Error::RateLimited {
            reset: Some(Utc.timestamp_opt(1795060800, 0).unwrap()),
        };
        assert_eq!(message(&e, &tz), "接口额度已用完，12:00 恢复");
    }
}
