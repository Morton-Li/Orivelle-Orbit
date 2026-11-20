use crate::error::Result;

/// Only fixed copy reaches the browser. Callback parameters and credentials stay native.
pub(super) fn response(result: &Result<()>) -> String {
    let (status, outcome, title, description, next, icon) = match result {
        Ok(()) => (
            "200 OK",
            "connected",
            "账户已连接",
            "授权已完成，Orbit 已连接你的 Orivelle 账户。",
            "回到 Orbit，即可查看账户、管理 API Key，或继续配置桌面工具。",
            r#"<path d="m5 12 4 4L19 6" />"#,
        ),
        Err(error) if error.code == "authorization_denied" => (
            "200 OK",
            "denied",
            "已取消授权",
            "你已拒绝此次连接，Orbit 未获得本次授权。",
            "如果需要连接账户，请回到 Orbit 重新发起授权。",
            r#"<path d="M6 12h12" />"#,
        ),
        Err(error) => (
            if error.code == "invalid_callback" {
                "400 Bad Request"
            } else {
                "502 Bad Gateway"
            },
            "failed",
            "连接未完成",
            "此次连接未能完成，请返回 Orbit 查看当前状态。",
            "请根据 Orbit 中的提示检查连接，或重新发起授权。",
            r#"<path d="m6 6 12 12M18 6 6 18" />"#,
        ),
    };
    let html = include_str!("authorization_callback.html")
        .replace("{{outcome}}", outcome)
        .replace("{{title}}", title)
        .replace("{{description}}", description)
        .replace("{{next}}", next)
        .replace("{{icon}}", icon);
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store, private\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    )
}
