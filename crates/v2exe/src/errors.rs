//! Library errors in Chinese, for the non-interactive output and the interface alike.

use chrono::FixedOffset;
use v2ex_rs::Error;

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
    fn rate_limited_message_uses_local_time() {
        let tz = FixedOffset::east_opt(8 * 3600).unwrap();
        let e = Error::RateLimited {
            reset: Some(Utc.timestamp_opt(1795060800, 0).unwrap()),
        };
        assert_eq!(message(&e, &tz), "接口额度已用完，12:00 恢复");
    }
}
