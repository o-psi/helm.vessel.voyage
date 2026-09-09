//! Strict origin validation for scoped Vessel connections.
use reqwest::Url;

pub fn validate_origin(origin: &str, allow_loopback_http: bool) -> anyhow::Result<String> {
    if origin.len() > 2048 || origin.chars().any(char::is_control) || origin.trim() != origin {
        return Err(anyhow::anyhow!("invalid Vessel origin"));
    }
    let url = Url::parse(origin).map_err(|_| anyhow::anyhow!("invalid Vessel origin"))?;
    // Require canonical literal IP text: URL parsing otherwise accepts 127.1,
    // integer/hex IPv4 and other surprising spellings as loopback addresses.
    let authority = origin
        .split_once("://")
        .map(|(_, s)| s.split('/').next().unwrap_or(""))
        .unwrap_or("");
    let literal = if authority.starts_with('[') {
        authority
            .split_once(']')
            .and_then(|(s, _)| s[1..].parse::<std::net::Ipv6Addr>().ok())
            .is_some_and(|ip| ip.is_loopback())
    } else {
        authority
            .split(':')
            .next()
            .and_then(|s| s.parse::<std::net::Ipv4Addr>().ok())
            .is_some_and(|ip| ip.is_loopback())
    };
    if !(url.scheme() == "https" || (url.scheme() == "http" && allow_loopback_http && literal))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || origin.contains('\\')
        || authority.contains('@')
    {
        return Err(anyhow::anyhow!("invalid Vessel origin"));
    }
    Ok(url.origin().ascii_serialization())
}
