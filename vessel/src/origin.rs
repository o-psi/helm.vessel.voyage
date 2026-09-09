/// No credentials, paths, queries, fragments, redirects, or non-TLS remote origins.
/// Development HTTP is allowed only for literal loopback IP addresses, not DNS.
pub fn validate_origin(origin: &str, allow_loopback_http: bool) -> anyhow::Result<String> {
    if origin.len() > 2048 || origin.chars().any(char::is_control) || origin.trim() != origin {
        anyhow::bail!("invalid public origin");
    }
    let url = url::Url::parse(origin).map_err(|_| anyhow::anyhow!("invalid public origin"))?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if !(url.scheme() == "https" || (url.scheme() == "http" && allow_loopback_http && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        anyhow::bail!("invalid public origin");
    }
    Ok(url.origin().ascii_serialization())
}
